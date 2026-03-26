use crate::{Backend, GatewayError, InputNode, Node, Route, RouteError, RoutePayload};
use http::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HOST};
use http::{HeaderName, StatusCode, Uri};
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::body::{Bytes, Frame};
use hyper::{Request, Response, server::conn::http1, service::service_fn};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioIo;
use llm_gateway_protocols::streaming::{self, StreamingCollector};
use llm_gateway_protocols::{Protocol, SseCollector, request};
use std::borrow::Cow;
use std::collections::HashSet;
use std::env;
use std::{fmt::Write, sync::Arc};
use tokio::net::TcpListener;

type BoxBody = http_body_util::combinators::BoxBody<Bytes, GatewayError>;
type HttpsClient = Client<HttpsConnector<HttpConnector>, Full<Bytes>>;

/// 运行 HTTP 服务器
pub async fn serve(input_node: &Arc<InputNode>) -> Result<(), GatewayError> {
    use std::net::SocketAddr;

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

/// 处理单个 HTTP 请求
async fn handle_request(
    req: Request<hyper::body::Incoming>,
    input_node: &InputNode,
    client: HttpsClient,
) -> Result<Response<BoxBody>, GatewayError> {
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

/// 直接转发请求到后端，支持 SSE 流式响应
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

    let mut converter: Box<dyn StreamingCollector> = match (protocol, backend.protocol) {
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
        Ok(response) => {
            let (parts, body) = response.into_parts();

            let mut collector = SseCollector::new();
            let mapped = body.map_frame(move |f| {
                let msgs = collector.collect(f.data_ref().unwrap()).unwrap();
                let mut ans = String::new();
                for msg in msgs {
                    log::debug!("in: {msg}");
                    if let Some(out) = converter.process(msg).unwrap() {
                        for line in out {
                            write!(ans, "{line}").unwrap()
                        }
                    }
                }
                log::debug!("out: {ans}");
                Frame::data(Bytes::from(ans))
            });

            // 流式转发后端响应体
            Ok(Response::from_parts(
                parts,
                mapped
                    .map_err(std::io::Error::other)
                    .map_err(GatewayError::IoError)
                    .boxed(),
            ))
        }
        Err(_) => Err(GatewayError::BackendRequestFailed(
            "Failed to connect to backend".into(),
        )),
    }
}
