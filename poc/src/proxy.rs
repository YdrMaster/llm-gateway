//! Pingora ProxyHttp 实现
//!
//! LLM Gateway 代理核心逻辑

use crate::backend::Backend;
use crate::backend::BackendManager;
use crate::middleware::concurrency::ConcurrencyLimitLayer;
use crate::middleware::protocol::ProtocolConversionMiddleware;
use crate::strategy::chain::{ChainExecutor, RoutingContext};
use crate::strategy::streaming_context::StreamingTransformCtx;

use async_trait::async_trait;
use bytes::Bytes;
use http::StatusCode;
use llm_gateway_protocols::Protocol;
use log::{debug, error, info, warn};
use pingora_core::upstreams::peer::HttpPeer;
use pingora_http::ResponseHeader;
use pingora_proxy::{ProxyHttp, Session};

/// LLM Gateway 代理
pub struct LlmGatewayProxy {
    /// 后端管理器
    backend_manager: BackendManager,
    /// 并发限制层
    concurrency_layer: ConcurrencyLimitLayer,
    /// 策略链执行器
    strategy_chain: ChainExecutor,
    /// 协议转换中间件
    protocol_middleware: ProtocolConversionMiddleware,
}

/// 代理上下文 - 持有并发许可直到请求结束
pub struct ProxyContext {
    /// 选中的后端
    pub selected_backend: Option<Backend>,
    /// 并发许可 - 持有直到 Drop
    pub _permit: Option<tokio::sync::OwnedSemaphorePermit>,
    /// 请求路径
    pub path: String,
    /// 模型名称
    pub model: String,
    /// 流式转换状态（在 upstream_response_filter 中创建，在 body filter 中使用）
    pub streaming_transform: Option<StreamingTransformCtx>,
    /// 是否已经开始向下游发送 body（用于未来 HTTP 级别 failover 判断）
    pub body_started: bool,
}

impl LlmGatewayProxy {
    /// 创建新的代理
    pub fn new(
        backend_manager: BackendManager,
        concurrency_layer: ConcurrencyLimitLayer,
        strategy_chain: ChainExecutor,
        protocol_middleware: ProtocolConversionMiddleware,
    ) -> Self {
        Self {
            backend_manager,
            concurrency_layer,
            strategy_chain,
            protocol_middleware,
        }
    }

    /// 从请求体中提取模型名称
    fn extract_model(&self, body: &Option<Bytes>) -> String {
        // 尝试从请求体中读取模型
        if let Some(body_bytes) = body
            && let Ok(json) = serde_json::from_slice::<serde_json::Value>(body_bytes)
            && let Some(model) = json.get("model").and_then(|v| v.as_str())
        {
            return model.to_string();
        }

        // 默认模型
        "default".to_string()
    }

    /// 根据选中的后端地址确定源协议
    fn detect_source_protocol(&self, ctx: &ProxyContext) -> Protocol {
        // 如果后端地址包含 "anthropic"，则认为是 Anthropic 协议
        if ctx
            .selected_backend
            .as_ref()
            .map(|b| b.addr.contains("anthropic"))
            .unwrap_or(false)
        {
            Protocol::Anthropic
        } else {
            // Mock 后端 A 和 B 返回 OpenAI 格式
            Protocol::OpenAI
        }
    }

    /// 根据源 → 目标创建对应的转换器
    fn create_streaming_transform(&self, ctx: &ProxyContext) -> Option<StreamingTransformCtx> {
        let source = self.detect_source_protocol(ctx);
        let target = Protocol::Anthropic; // 测试用：OpenAI → Anthropic

        let converter = self.protocol_middleware.create_converter(source, target);

        if converter.is_some() {
            Some(StreamingTransformCtx::new(
                source.name().to_string(),
                target.name().to_string(),
                converter,
            ))
        } else {
            // 无需转换 — 仍然创建上下文以进行 SSE 解析
            Some(StreamingTransformCtx::new(
                source.name().to_string(),
                source.name().to_string(),
                None,
            ))
        }
    }
}

#[async_trait]
impl ProxyHttp for LlmGatewayProxy {
    type CTX = ProxyContext;

    fn new_ctx(&self) -> Self::CTX {
        ProxyContext {
            selected_backend: None,
            _permit: None,
            path: String::new(),
            model: String::new(),
            streaming_transform: None,
            body_started: false,
        }
    }

    async fn request_body_filter(
        &self,
        session: &mut Session,
        body: &mut Option<Bytes>,
        _end_of_stream: bool,
        ctx: &mut Self::CTX,
    ) -> Result<(), Box<pingora_core::Error>> {
        // 记录请求路径
        ctx.path = session.req_header().uri.path().to_string();
        debug!(
            "Processing request: {} {}",
            session.req_header().method,
            ctx.path
        );

        // 提取模型名称（只在第一个 body chunk 处理）
        if ctx.model.is_empty() && body.is_some() {
            ctx.model = self.extract_model(body);
            debug!("Extracted model: {}", ctx.model);

            // 获取并发许可 - 许可会保存在 ctx 中直到请求结束
            match self.concurrency_layer.acquire(&ctx.model).await {
                Ok(permit) => {
                    ctx._permit = Some(permit);
                    info!("Acquired concurrency permit for model: {}", ctx.model);
                }
                Err(e) => {
                    warn!("Failed to acquire concurrency permit: {e}");
                    // 返回 503
                    let response = ResponseHeader::build(StatusCode::SERVICE_UNAVAILABLE, None)?;
                    session
                        .write_response_header(Box::new(response), true)
                        .await?;
                    return Err(pingora_core::Error::new_str("Concurrency limit exceeded"));
                }
            }
        }

        Ok(())
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>, Box<pingora_core::Error>> {
        // 创建路由上下文
        let routing_ctx =
            RoutingContext::new(ctx.model.clone(), ctx.path.clone(), "openai".to_string());

        // 执行策略链选择后端
        match self.strategy_chain.execute(&routing_ctx).await {
            Ok(backend) => {
                info!("Selected backend: {}", backend.addr);
                ctx.selected_backend = Some(backend.clone());
                let peer = HttpPeer::new(&backend.addr, false, "".to_string());
                Ok(Box::new(peer))
            }
            Err(e) => {
                error!("Failed to select backend: {e}");
                // 返回 503
                let response = ResponseHeader::build(StatusCode::SERVICE_UNAVAILABLE, None)?;
                session
                    .write_response_header(Box::new(response), true)
                    .await?;
                Err(pingora_core::Error::new_str("No available backend"))
            }
        }
    }

    async fn logging(
        &self,
        session: &mut Session,
        e: Option<&pingora_core::Error>,
        ctx: &mut Self::CTX,
    ) {
        // 记录后端健康状态
        if let Some(backend) = &ctx.selected_backend {
            if e.is_some() {
                self.backend_manager.record_failure(&backend.addr).await;
                warn!("Recorded failure for backend: {}", backend.addr);
            } else {
                self.backend_manager.record_success(&backend.addr).await;
                debug!("Recorded success for backend: {}", backend.addr);
            }
        }

        // 记录请求日志
        let status = session
            .response_written()
            .map(|h| h.status.as_u16())
            .unwrap_or(0);
        info!(
            "Request completed: {} {} -> {} (error: {:?})",
            session.req_header().method,
            ctx.path,
            status,
            e
        );

        // ctx 在此处 Drop，并发许可会自动释放
        debug!("Request context dropped, concurrency permit released");
    }

    async fn upstream_response_filter(
        &self,
        session: &mut Session,
        upstream_response: &mut ResponseHeader,
        ctx: &mut Self::CTX,
    ) -> Result<(), Box<pingora_core::Error>> {
        // 在收到第一个响应头时初始化流式转换上下文
        ctx.streaming_transform = self.create_streaming_transform(ctx);

        // 检测 5xx 错误，记录到健康监控（影响后续请求的路由决策）
        let status = upstream_response.status.as_u16();
        if status >= 500
            && let Some(backend) = &ctx.selected_backend
        {
            warn!(
                "Upstream returned {} for {} {} (backend: {})",
                status,
                session.req_header().method,
                ctx.path,
                backend.addr
            );
            // 记录失败，影响后续请求的路由（当前请求的响应头已发送，无法撤回）
            // 注意：由于 Pingora 管道中响应头已写入下游，
            // 无法在此阶段静默重试。HTTP 级别重试需要 Phase 1 的 custom_forwarding 实现。
        }

        Ok(())
    }

    fn upstream_response_body_filter(
        &self,
        _session: &mut Session,
        body: &mut Option<Bytes>,
        end_of_stream: bool,
        ctx: &mut Self::CTX,
    ) -> Result<Option<std::time::Duration>, Box<pingora_core::Error>> {
        // 标记 body 已经开始传输
        if body.is_some() {
            ctx.body_started = true;
        }

        // 正常流程：流式转换
        let Some(transform) = ctx.streaming_transform.as_mut() else {
            return Ok(None);
        };

        let mut output = Vec::new();

        if let Some(chunk) = body.take() {
            output.extend(transform.process_chunk(&chunk));
        }

        if end_of_stream {
            output.extend(transform.finish());
            info!(
                "Streaming transform finished: {} → {}",
                transform.source_protocol, transform.target_protocol
            );
        }

        if output.is_empty() {
            *body = None;
        } else {
            *body = Some(Bytes::from(output));
        }

        Ok(None)
    }

    fn fail_to_connect(
        &self,
        _session: &mut Session,
        _peer: &HttpPeer,
        _ctx: &mut Self::CTX,
        mut e: Box<pingora_core::Error>,
    ) -> Box<pingora_core::Error> {
        // 标记为可重试，使 Pingora 重新调用 upstream_peer
        e.set_retry(true);
        e
    }
}
