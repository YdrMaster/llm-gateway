use std::process::Child;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

/// 为每个测试分配唯一端口，避免并行测试冲突
static NEXT_PORT: AtomicU16 = AtomicU16::new(28080);

pub fn allocate_port() -> u16 {
    NEXT_PORT.fetch_add(1, Ordering::SeqCst)
}

/// 启动代理二进制并等待就绪。
/// 返回代理监听的端口。
pub fn start_proxy() -> (Child, u16) {
    let port = allocate_port();

    // SAFETY: 测试环境中单线程设置环境变量是安全的
    unsafe {
        std::env::set_var("POC_PROXY_PORT", port.to_string());
    }

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_llm-gateway-poc"))
        .env("RUST_LOG", "warn")
        .spawn()
        .expect("Failed to start proxy binary");

    // 等待代理就绪
    for _ in 0..50 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return (child, port);
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let _ = child.kill();
    panic!("Proxy did not start within 5 seconds on port {port}");
}

/// 停止代理子进程
pub fn stop_proxy(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait_with_output();
}

/// 创建测试用 HTTP 客户端
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client")
}
