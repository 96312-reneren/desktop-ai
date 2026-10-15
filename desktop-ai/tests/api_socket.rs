//! API 服务器 socket 级测试（红队第 2 轮 · 差距 #1）。
//!
//! 与 `api_server.rs` 内联的 25 个解析器用例不同，这里从 **真实 TCP 连接**
//! 打到 `ApiServer`：
//!   - 网络读取层（read_http_request 三阶段：头区上限 / Content-Length 上限 /
//!     慢速客户端超时）；
//!   - 并发准入（MAX_CONCURRENT_CONNS=16 的 CAS 槽位与回收）；
//!   - 认证与路由全链路（Bearer 401 / CORS / JSON 校验 / 503 模型未加载）。
//!
//! 测试桩不加载模型：`ApiServer::start(None, …)`（2026-10 引入的 Option 化
//! 参数）+ 端口 0（OS 分配临时端口，`port()` 回传实际值），每个用例独立
//! server，避免用例间争抢固定端口。
//!
//! 所有连接都指向 127.0.0.1，零外联流量。

use desktop_ai::ApiServer;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// 启动无模型桩，返回 (server, 实际端口)。
fn start_stub() -> (ApiServer, u16) {
    let server = ApiServer::start(None, 0, "stub-model".into(), "test-token".into());
    let port = server.port().expect("API server bind failed");
    (server, port)
}

/// 发送原始字节，读到对端关闭（服务器恒 Connection: close）。
fn send_raw(port: u16, raw: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(raw.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

fn status_of(resp: &str) -> u16 {
    resp.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn body_of(resp: &str) -> &str {
    resp.split("\r\n\r\n").nth(1).unwrap_or("")
}

fn get(port: u16, path: &str) -> String {
    send_raw(
        port,
        &format!("GET {} HTTP/1.1\r\nHost: localhost\r\n\r\n", path),
    )
}

fn post_chat(port: u16, token: Option<&str>, body: &str) -> String {
    let mut req = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n"
    );
    if let Some(t) = token {
        req.push_str(&format!("Authorization: Bearer {}\r\n", t));
    }
    req.push_str(&format!("Content-Length: {}\r\n\r\n{}", body.len(), body));
    send_raw(port, &req)
}

// ─── 基础路由（无认证） ─────────────────────────────────────

#[test]
fn health_and_ready_unauthenticated() {
    let (_server, port) = start_stub();
    let h = get(port, "/health");
    assert_eq!(status_of(&h), 200);
    assert!(body_of(&h).contains("ok"), "health body: {}", body_of(&h));
    let r = get(port, "/ready");
    assert_eq!(status_of(&r), 200);
    assert!(body_of(&r).contains("ready"), "ready body: {}", body_of(&r));
}

#[test]
fn root_welcome_page() {
    let (_server, port) = start_stub();
    let resp = get(port, "/");
    assert_eq!(status_of(&resp), 200);
    assert!(
        body_of(&resp).contains("endpoints"),
        "root body: {}",
        body_of(&resp)
    );
}

// ─── 认证链路 ───────────────────────────────────────────────

#[test]
fn v1_requires_bearer_token() {
    let (_server, port) = start_stub();
    // 无 Authorization
    let no_auth = get(port, "/v1/models");
    assert_eq!(
        status_of(&no_auth),
        401,
        "无 token 的 /v1/* 必须 401: {}",
        no_auth
    );
    // 错误 token
    let bad = get_with_token(port, "/v1/models", "wrong-token");
    assert_eq!(status_of(&bad), 401, "错误 token 必须 401: {}", bad);
}

fn get_with_token(port: u16, path: &str, token: &str) -> String {
    send_raw(
        port,
        &format!(
            "GET {} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\n\r\n",
            path, token
        ),
    )
}

#[test]
fn v1_models_with_token_lists_stub() {
    let (_server, port) = start_stub();
    let resp = get_with_token(port, "/v1/models", "test-token");
    assert_eq!(status_of(&resp), 200, "{}", resp);
    assert!(
        body_of(&resp).contains("stub-model"),
        "models body: {}",
        body_of(&resp)
    );
}

// ─── CORS ──────────────────────────────────────────────────

#[test]
fn options_preflight_reflects_allowed_origin() {
    let (_server, port) = start_stub();
    let resp = send_raw(
        port,
        "OPTIONS /v1/chat/completions HTTP/1.1\r\nOrigin: http://localhost:3000\r\n\r\n",
    );
    assert_eq!(status_of(&resp), 204, "{}", resp);
    assert!(
        resp.contains("Access-Control-Allow-Origin: http://localhost:3000"),
        "预检应反射白名单 origin: {}",
        resp
    );
}

#[test]
fn disallowed_origin_rejected_403() {
    let (_server, port) = start_stub();
    let resp = send_raw(
        port,
        "GET /health HTTP/1.1\r\nOrigin: http://evil.example\r\n\r\n",
    );
    assert_eq!(status_of(&resp), 403, "{}", resp);
}

// ─── /v1/chat/completions 校验链（桩无模型 → 最终 503） ──────

#[test]
fn chat_invalid_json_400() {
    let (_server, port) = start_stub();
    let resp = post_chat(port, Some("test-token"), "{not json");
    assert_eq!(status_of(&resp), 400, "{}", resp);
    assert!(
        body_of(&resp).contains("invalid_json"),
        "错误响应应带机器可读 code: {}",
        body_of(&resp)
    );
}

#[test]
fn chat_missing_messages_400() {
    let (_server, port) = start_stub();
    let resp = post_chat(port, Some("test-token"), r#"{"model":"x"}"#);
    assert_eq!(status_of(&resp), 400, "{}", resp);
    assert!(
        body_of(&resp).contains("missing_messages"),
        "body: {}",
        body_of(&resp)
    );
}

#[test]
fn chat_invalid_role_400() {
    let (_server, port) = start_stub();
    let body = r#"{"model":"x","messages":[{"role":"tool","content":"hi"}]}"#;
    let resp = post_chat(port, Some("test-token"), body);
    assert_eq!(status_of(&resp), 400, "{}", resp);
    assert!(
        body_of(&resp).contains("invalid_role"),
        "body: {}",
        body_of(&resp)
    );
}

/// 6.4 万字符（>64K 字符上限）→ validate_messages 拒绝。
#[test]
fn chat_content_too_long_chars_400() {
    let (_server, port) = start_stub();
    let content = "a".repeat(70_000);
    let body = format!(
        r#"{{"model":"x","messages":[{{"role":"user","content":"{}"}}]}}"#,
        content
    );
    let resp = post_chat(port, Some("test-token"), &body);
    assert_eq!(status_of(&resp), 400, "{}", resp);
    assert!(
        body_of(&resp).contains("content_too_long"),
        "body: {}",
        body_of(&resp)
    );
}

/// 字符数 ≤64K 但字节 >64K（多字节中文）→ 单条消息字节上限拒绝。
#[test]
fn chat_content_too_large_bytes_400() {
    let (_server, port) = start_stub();
    let content = "中".repeat(33_000); // 33K 字符 = 99KB UTF-8
    let body = format!(
        r#"{{"model":"x","messages":[{{"role":"user","content":"{}"}}]}}"#,
        content
    );
    assert!(body.len() < 1_048_576, "测试载荷应小于请求体上限");
    let resp = post_chat(port, Some("test-token"), &body);
    assert_eq!(status_of(&resp), 400, "{}", resp);
    assert!(
        body_of(&resp).contains("content_too_large"),
        "body: {}",
        body_of(&resp)
    );
}

/// 全部 400 校验通过 + 桩未加载模型 → 503 model_not_loaded。
/// 这是 Option 化 inf 的核心行为：无模型的服务器对 otherwise-valid 请求
/// 明确报"模型未加载"，而不是静默或 panic。
#[test]
fn chat_valid_without_model_503() {
    let (_server, port) = start_stub();
    let body = r#"{"model":"x","messages":[{"role":"user","content":"hi"}]}"#;
    let resp = post_chat(port, Some("test-token"), body);
    assert_eq!(status_of(&resp), 503, "{}", resp);
    assert!(
        body_of(&resp).contains("model_not_loaded"),
        "body: {}",
        body_of(&resp)
    );
}

/// 无 token 时 chat 请求 401（校验发生在路由/模型检查之前）。
#[test]
fn chat_without_token_401_even_when_model_missing() {
    let (_server, port) = start_stub();
    let body = r#"{"model":"x","messages":[{"role":"user","content":"hi"}]}"#;
    let resp = post_chat(port, None, body);
    assert_eq!(status_of(&resp), 401, "{}", resp);
}

// ─── 网络读取层（read_http_request 三阶段） ──────────────────

/// 声明 2MiB Content-Length 但不发 body：必须在读取阶段 2 直接拒绝，
/// 不得把 2MiB 读进内存（请求体上限 1MiB）。
#[test]
fn oversized_content_length_rejected_without_body() {
    let (_server, port) = start_stub();
    let resp = send_raw(
        port,
        "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2097152\r\n\r\n",
    );
    assert_eq!(status_of(&resp), 400, "{}", resp);
}

/// 9000 字节 URI（> 8192 上限）→ 414。
#[test]
fn uri_too_long_414() {
    let (_server, port) = start_stub();
    let long = "/".to_string() + &"a".repeat(9000);
    let resp = get(port, &long);
    assert_eq!(status_of(&resp), 414, "{}", resp);
}

/// 70 个请求头（> 64 上限）→ 431。
#[test]
fn too_many_headers_431() {
    let (_server, port) = start_stub();
    let mut req = String::from("GET / HTTP/1.1\r\nHost: localhost\r\n");
    for i in 0..70 {
        req.push_str(&format!("X-H{}: v\r\n", i));
    }
    req.push_str("\r\n");
    let resp = send_raw(port, &req);
    assert_eq!(status_of(&resp), 431, "{}", resp);
}

/// 白名单外方法 → 405。
#[test]
fn method_not_allowed_405() {
    let (_server, port) = start_stub();
    let resp = send_raw(port, "PUT / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert_eq!(status_of(&resp), 405, "{}", resp);
}

/// 版本不是 1.0/1.1 → 505。
#[test]
fn unsupported_version_505() {
    let (_server, port) = start_stub();
    let resp = send_raw(port, "GET / HTTP/9.9\r\nHost: localhost\r\n\r\n");
    assert_eq!(status_of(&resp), 505, "{}", resp);
}

/// 双 Content-Length（请求走私经典面）→ 400。
#[test]
fn duplicate_content_length_400() {
    let (_server, port) = start_stub();
    let resp = send_raw(
        port,
        "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\nx",
    );
    assert_eq!(status_of(&resp), 400, "{}", resp);
}

// ─── 并发准入（MAX_CONCURRENT_CONNS=16） ─────────────────────

/// 16 个"只连不发"的连接占满槽位（30s read timeout 期间各占一槽），
/// 第 17 个连接立即收到 503；全部断开后槽位回收，/health 恢复 200。
/// 回收依赖 handler 读到 EOF→400→fetch_sub——证明拒绝是瞬时的、
/// 不存在"打满一次就永久锁死"的 DoS。
#[test]
fn concurrent_cap_503_then_recovery() {
    let (server, port) = start_stub();

    let mut holders: Vec<TcpStream> = (0..16)
        .map(|_| TcpStream::connect(("127.0.0.1", port)).expect("holder connect"))
        .collect();

    // 第 17 个连接应立即收到 503
    let mut seventeenth = TcpStream::connect(("127.0.0.1", port)).expect("17th connect");
    seventeenth
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut buf = vec![0u8; 1024];
    let n = seventeenth.read(&mut buf).unwrap_or(0);
    let resp = String::from_utf8_lossy(&buf[..n]).into_owned();
    assert_eq!(status_of(&resp), 503, "第 17 个连接应 503: {}", resp);

    // 全部断开后，槽位应回收，/health 在 5 秒内恢复
    holders.clear();
    let mut recovered = false;
    for _ in 0..50 {
        let resp = get(port, "/health");
        if status_of(&resp) == 200 {
            recovered = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(recovered, "释放连接后 /health 应在 5s 内恢复 200");
    drop(server); // 提前停止，避免监听线程活到测试进程结束
}
