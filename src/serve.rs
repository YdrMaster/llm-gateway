use crate::{Backend, GatewayError, InputNode, Node, Route, RouteError, RoutePayload};
use http::header::{ALLOW, AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HOST};
use http::{HeaderName, Method, StatusCode, Uri};
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::body::{Bytes, Frame, Incoming};
use hyper::{Request, Response, server::conn::http1, service::service_fn};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioIo;
use llm_gateway_protocols::streaming::{self, StreamingCollector};
use llm_gateway_protocols::{Protocol, SseCollector, SseMessage, request};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::collections::HashSet;
use std::env;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fmt::Write, sync::Arc};
use tokio::net::TcpListener;

type BoxBody = http_body_util::combinators::BoxBody<Bytes, GatewayError>;
type HttpsClient = Client<HttpsConnector<HttpConnector>, Full<Bytes>>;

/// 运行 HTTP 服务器
pub async fn serve(input_node: &Arc<InputNode>) -> Result<(), GatewayError> {
    let addr = SocketAddr::from(([0, 0, 0, 0], input_node.port));
    let listener = TcpListener::bind(addr).await?;
    let client = client();

    log::info!("Listening on {addr}");

    loop {
        let (stream, remote_addr) = listener.accept().await?;
        log::info!("Accepted connection from {remote_addr}");

        let node = input_node.clone();
        let client = client.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |req| {
                let node = node.clone();
                let client = client.clone();
                async move { handle_request(req, &node, client).await }
            });

            if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                log::warn!("Error handling connection from {remote_addr}: {e}");
            }
        });
    }
}

fn client() -> HttpsClient {
    // 创建支持 HTTP 和 HTTPS 的连接器
    let mut http_connector = HttpConnector::new();
    http_connector.set_nodelay(true);
    http_connector.enforce_http(false);

    let https_connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()
        .unwrap()
        .https_or_http()
        .enable_http1()
        .wrap_connector(http_connector);

    Client::builder(hyper_util::rt::TokioExecutor::new())
        .pool_max_idle_per_host(32)
        .build(https_connector)
}

fn env_key(key: &str) -> Cow<'_, str> {
    key.strip_prefix("$")
        .and_then(|key| env::var(key).ok())
        .map(Cow::Owned)
        .unwrap_or(Cow::Borrowed(key))
}

fn method_not_allowed(allow: Method) -> Response<BoxBody> {
    let allow = allow.as_str();
    Response::builder()
        .status(StatusCode::METHOD_NOT_ALLOWED)
        .header(ALLOW, allow)
        .body(
            Full::<Bytes>::from(format!("Method not allowed. Use {allow}."))
                .map_err(|_| GatewayError::NoAvailableBackend)
                .boxed(),
        )
        .unwrap()
}

/// 处理单个 HTTP 请求
async fn handle_request(
    req: Request<hyper::body::Incoming>,
    input_node: &InputNode,
    client: HttpsClient,
) -> Result<Response<BoxBody>, GatewayError> {
    // 处理 /v1/models 端点 (GET only)
    if req.uri().path() == "/v1/models" {
        return if req.method() != Method::GET {
            Ok(method_not_allowed(Method::GET))
        } else {
            handle_models_request(input_node).await
        };
    }
    if req.method() != Method::POST {
        return Ok(method_not_allowed(Method::POST));
    }

    let payload = RoutePayload::new(req).await?;
    match input_node.route(&payload) {
        Ok(Route { mut nodes, backend }) => {
            // 日志记录路由成功路径
            nodes.reverse();
            let path_str = nodes.join(" -> ");
            log::info!("Routing path: {path_str}");

            if payload.protocol() == backend.protocol {
                forward_to_backend(payload, backend, client).await
            } else {
                forward_to_foreign(payload, backend, client).await
            }
        }
        Err(e) => match e {
            RouteError::NoAvailable => {
                log::warn!("No available backend for this model");
                Ok(Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(
                        Full::<Bytes>::from("No available backend for this model")
                            .map_err(|_| GatewayError::NoAvailableBackend)
                            .boxed(),
                    )
                    .unwrap())
            }
        },
    }
}

/// 处理 /v1/models 请求
async fn handle_models_request(input_node: &InputNode) -> Result<Response<BoxBody>, GatewayError> {
    let models: Vec<Value> = input_node
        .models
        .read()
        .unwrap()
        .keys()
        .map(|name| {
            json!({
                "id": name.as_ref(),
                "object": "model",
                "created": 0,
                "owned_by": "llm-gateway"
            })
        })
        .collect();

    let response = json!({
        "object": "list",
        "data": models
    });

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(
            Full::<Bytes>::from(response.to_string())
                .map_err(|_| GatewayError::NoAvailableBackend)
                .boxed(),
        )
        .unwrap())
}

const X_API_KEY: HeaderName = HeaderName::from_static("x-api-key");
const ANTHROPIC_VERSION: HeaderName = HeaderName::from_static("anthropic-version");

/// 直接转发请求到后端，支持 SSE 流式响应
async fn forward_to_backend(
    payload: RoutePayload,
    backend: Backend,
    client: HttpsClient,
) -> Result<Response<BoxBody>, GatewayError> {
    // 重建 URI
    let uri = format!("{}{}", backend.base_url, payload.parts.uri.path())
        .parse::<Uri>()
        .map_err(|e| GatewayError::BackendRequestFailed(e.to_string()))?;

    // 重建请求
    let mut req_builder = Request::builder()
        .method(payload.parts.method)
        .uri(uri)
        .header(CONTENT_TYPE, "application/json");

    let mut skip_header = HashSet::from([HOST, CONTENT_TYPE, CONTENT_LENGTH]);

    if let Some(api_key) = backend.api_key.as_deref() {
        let mut api_key_added = false;
        if payload.parts.headers.contains_key(X_API_KEY) {
            req_builder = req_builder.header(X_API_KEY, api_key);
            skip_header.insert(X_API_KEY);
            api_key_added = true
        }
        if payload.parts.headers.contains_key(AUTHORIZATION) {
            req_builder = req_builder.header(AUTHORIZATION, api_key);
            skip_header.insert(AUTHORIZATION);
            api_key_added = true
        }
        if !api_key_added {
            let api_key = env_key(api_key);
            match backend.protocol {
                Protocol::OpenAI => req_builder = req_builder.header(AUTHORIZATION, &*api_key),
                Protocol::Anthropic => req_builder = req_builder.header("x-api-key", &*api_key),
            }
        }
    }

    // 转发所有原始 headers
    for (name, value) in payload.parts.headers {
        if let Some(name) = name
            && !skip_header.contains(&name)
        {
            req_builder = req_builder.header(name, value)
        }
    }

    log::debug!("use headers: {:#?}", req_builder.headers_ref());
    let forward_req: Request<Full<Bytes>> = req_builder
        .body(Full::from(serde_json::to_vec(&payload.body).unwrap()))
        .unwrap();

    // 发送请求到后端
    match client.request(forward_req).await {
        Ok(response) => {
            let (parts, body) = response.into_parts();

            // 流式转发后端响应体
            Ok(Response::from_parts(
                parts,
                body.map_err(std::io::Error::other)
                    .map_err(GatewayError::IoError)
                    .boxed(),
            ))
        }
        Err(_) => Err(GatewayError::BackendRequestFailed(
            "Failed to connect to backend".into(),
        )),
    }
}

async fn forward_to_foreign(
    payload: RoutePayload,
    backend: Backend,
    client: HttpsClient,
) -> Result<Response<BoxBody>, GatewayError> {
    let protocol = payload.protocol();
    log::info!("forward to foreign: {protocol:?} -> {:?}", backend.protocol);

    // 重建 URI
    let uri = format!("{}{}", backend.base_url, backend.protocol.path())
        .parse::<Uri>()
        .map_err(|e| GatewayError::BackendRequestFailed(e.to_string()))?;

    // 重建请求
    let mut req_builder = Request::builder()
        .method(payload.parts.method)
        .uri(uri)
        .header(CONTENT_TYPE, "application/json");

    let mut skip_header =
        HashSet::from([HOST, CONTENT_TYPE, CONTENT_LENGTH, AUTHORIZATION, X_API_KEY]);

    if matches!(backend.protocol, Protocol::OpenAI) {
        skip_header.insert(ANTHROPIC_VERSION);
    }

    if let Some(api_key) = backend.api_key.as_deref() {
        let api_key = env_key(api_key);
        match backend.protocol {
            Protocol::OpenAI => req_builder = req_builder.header(AUTHORIZATION, &*api_key),
            Protocol::Anthropic => req_builder = req_builder.header(X_API_KEY, &*api_key),
        }
    }

    // 转发所有原始 headers
    for (name, value) in payload.parts.headers {
        if let Some(name) = name
            && !skip_header.contains(&name)
        {
            req_builder = req_builder.header(name, value)
        }
    }

    // 协议转换
    let body = match (protocol, backend.protocol) {
        (Protocol::OpenAI, Protocol::Anthropic) => {
            request::openai_to_anthropic(payload.body).unwrap()
        }
        (Protocol::Anthropic, Protocol::OpenAI) => {
            request::anthropic_to_openai(payload.body).unwrap()
        }
        (_, _) => unreachable!(),
    };

    log::debug!("use headers: {:#?}", req_builder.headers_ref());
    let forward_req: Request<Full<Bytes>> = req_builder
        .body(Full::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let converter: Box<dyn StreamingCollector> = match (protocol, backend.protocol) {
        (Protocol::OpenAI, Protocol::Anthropic) => {
            Box::new(streaming::AnthropicToOpenai::default())
        }
        (Protocol::Anthropic, Protocol::OpenAI) => {
            Box::new(streaming::OpenaiToAnthropic::default())
        }
        (_, _) => unreachable!(),
    };

    // 发送请求到后端
    match client.request(forward_req).await {
        Ok(response) => Ok(forward_foreign_response(response, converter)),
        Err(_) => Err(GatewayError::BackendRequestFailed(
            "Failed to connect to backend".into(),
        )),
    }
}

/// 使用 Stream 方式处理协议转换，错误时真正关闭流
fn forward_foreign_response(
    response: Response<Incoming>,
    converter: Box<dyn StreamingCollector>,
) -> Response<BoxBody> {
    use futures::{StreamExt, TryStreamExt};

    let (parts, body) = response.into_parts();

    // 将 Body 转换为 Stream，将 hyper::Error 映射为 std::io::Error
    let data_stream = body.into_data_stream().map_err(std::io::Error::other);

    // 使用 Mutex 实现线程安全的内部可变性
    let collector = Mutex::new(SseCollector::new());
    let converter = Mutex::new(converter);
    let error_occurred = Arc::new(AtomicBool::new(false));

    // 创建一个 Stream，在错误时立即停止
    let processed_stream = data_stream
        .try_take_while({
            let error_occurred = error_occurred.clone();
            move |_| {
                let should_continue = !error_occurred.load(Ordering::Relaxed);
                futures::future::ready(Ok::<_, std::io::Error>(should_continue))
            }
        })
        .map({
            let error_occurred = error_occurred.clone();
            move |result| match result {
                Ok(bytes) => Ok(process_frame(
                    &bytes,
                    &collector,
                    &converter,
                    &error_occurred,
                )),
                Err(e) => Err(e),
            }
        });

    // 将 Stream 转换回 Body
    let new_body = http_body_util::StreamBody::new(processed_stream);
    let new_body = BodyExt::map_err(new_body, GatewayError::IoError);

    Response::from_parts(parts, BodyExt::boxed(new_body))
}

/// 处理单个数据帧，返回 SSE 格式的输出
fn process_frame(
    bytes: &Bytes,
    collector: &Mutex<SseCollector>,
    converter: &Mutex<Box<dyn StreamingCollector>>,
    error_occurred: &AtomicBool,
) -> Frame<Bytes> {
    let ans = match collector.lock().unwrap().collect(bytes) {
        Ok(msgs) => {
            let mut ans = String::new();
            for msg in msgs {
                log::debug!("in: {msg}");
                let mut converter = converter.lock().unwrap();
                match converter.process(msg) {
                    Ok(out) => {
                        for line in out {
                            let _ = write!(ans, "{line}");
                        }
                    }
                    Err(e) => {
                        log::error!("Protocol conversion error: {e}");
                        error_occurred.store(true, Ordering::Relaxed);
                        let error_data = serde_json::json!({
                            "error": {
                                "message": format!("Protocol conversion failed: {e}"),
                                "type": "protocol_conversion_error"
                            }
                        });
                        let _ = write!(ans, "{}", SseMessage::new(&error_data));
                        break;
                    }
                }
            }
            ans
        }
        Err(e) => {
            log::error!("SSE parsing error: {e}");
            error_occurred.store(true, Ordering::Relaxed);
            let error_data = serde_json::json!({
                "error": {
                    "message": format!("SSE parsing failed: {e}"),
                    "type": "sse_parsing_error"
                }
            });
            SseMessage::new(&error_data).to_string()
        }
    };
    log::debug!("out: {ans}");
    Frame::data(Bytes::from(ans))
}
