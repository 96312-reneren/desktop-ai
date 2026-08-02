use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex,
};
use std::thread;

use crate::inference::{LlamaInference, StreamToken};

/// 常量时间字符串比较，防止时序攻击
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let max_len = a.len().max(b.len());
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..max_len {
        let x = *a.get(i).unwrap_or(&0);
        let y = *b.get(i).unwrap_or(&0);
        diff |= x ^ y;
    }
    diff == 0
}

/// Maximum simultaneous in-flight API connections. Extra connections are
/// rejected with 503 to prevent trivial local DoS via unbounded thread spawn.
const MAX_CONCURRENT_CONNS: u32 = 16;
/// Hard cap on request body size (1 MiB) to bound memory per request.
const MAX_BODY_SIZE: usize = 1_048_576;
/// Hosts permitted by the CORS policy. Command-line clients (no `Origin`
/// header) are always allowed; browser origins must match one of these.
const ALLOWED_ORIGIN_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];
/// Allowed HTTP methods for CORS preflight responses.
const CORS_ALLOWED_METHODS: &str = "GET, POST";
/// Allowed request headers for CORS preflight responses.
const CORS_ALLOWED_HEADERS: &str = "Content-Type, Authorization";
/// Preflight cache duration in seconds (1 hour).
const CORS_MAX_AGE: &str = "3600";
/// 最大请求 URI 长度（8 KiB），防止超长 URI DoS。
const MAX_URI_LEN: usize = 8192;
/// 最大请求头数量，防止头注入 DoS。
const MAX_HEADER_COUNT: usize = 64;
/// 单个请求头值最大长度（8 KiB）。
const MAX_HEADER_VALUE_LEN: usize = 8192;
/// 请求头区域最大总字节数（64 KiB）。
const MAX_HEADER_SECTION_SIZE: usize = 65536;
/// 允许的 HTTP 方法白名单。
const ALLOWED_METHODS: &[&str] = &["GET", "POST", "OPTIONS"];

pub struct ApiServer {
    stop_flag: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl ApiServer {
    pub fn start(
        inf: Arc<Mutex<LlamaInference>>,
        port: u16,
        active_model: String,
        api_token: String,
    ) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop = stop_flag.clone();
        let active_conns = Arc::new(AtomicU32::new(0));

        let handle = thread::spawn(move || {
            let addr = format!("127.0.0.1:{}", port);
            let listener = match TcpListener::bind(&addr) {
                Ok(l) => {
                    log::info!("API server listening on http://{}", addr);
                    l
                }
                Err(e) => {
                    log::error!("API server bind failed: {}", e);
                    return;
                }
            };
            if let Err(e) = listener.set_nonblocking(true) {
                log::warn!("API server set_nonblocking failed: {}", e);
            }

            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        // Atomic fetch_update avoids TOCTOU: two threads that
                        // both see `cur=15` would otherwise both pass the
                        // `cur >= MAX` check and double-increment.
                        let result = active_conns.fetch_update(
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                            |cur| {
                                if cur >= MAX_CONCURRENT_CONNS {
                                    None
                                } else {
                                    Some(cur + 1)
                                }
                            },
                        );
                        if result.is_err() {
                            let mut s = stream;
                            let _ = s.write_all(
                                b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            );
                            continue;
                        }
                        let inf = Arc::clone(&inf);
                        let model_name = active_model.clone();
                        let conns = Arc::clone(&active_conns);
                        let token = api_token.clone();
                        thread::spawn(move || {
                            handle_client(stream, inf, model_name, &token);
                            conns.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(100));
                    }
                    Err(_) => break,
                }
            }
            log::info!("API server stopped");
        });

        ApiServer {
            stop_flag,
            handle: Some(handle),
        }
    }

    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for ApiServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn handle_client(
    mut stream: TcpStream,
    inf: Arc<Mutex<LlamaInference>>,
    model_name: String,
    api_token: &str,
) {
    // Guard against slow-loris: a client sending 1 byte / minute would
    // otherwise hold a connection slot indefinitely. The per-read timeout
    // bounds each syscall; the overall deadline bounds the whole request.
    if let Err(e) = stream.set_read_timeout(Some(std::time::Duration::from_secs(30))) {
        log::warn!("API set_read_timeout failed: {}", e);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let raw = match read_http_request(&mut stream, deadline) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("API read_http_request: {}", e);
            let _ = stream.write_all(json_error(400, "bad_request", "bad request").as_bytes());
            return;
        }
    };
    let request = String::from_utf8_lossy(&raw);

    // 安全加固：使用经过验证的 HTTP 解析器
    let parsed = match parse_http_validated(&request) {
        Ok(p) => p,
        Err((code, msg)) => {
            log::warn!("API parse_http 拒绝: {} (状态码 {})", msg, code);
            let _ = stream.write_all(json_error(code, "malformed_request", msg).as_bytes());
            return;
        }
    };
    let method = parsed.method.as_str();
    let path = parsed.path;
    let body = parsed.body;
    let origin = parsed.origin;

    // CORS: browser origins must be on the allow-list; command-line (no
    // Origin) is always permitted.
    if let Some(ref origin) = origin {
        if !origin_allowed(origin) {
            log::warn!("API rejected Origin: {}", origin);
            let _ = stream
                .write_all(json_error(403, "origin_not_allowed", "origin not allowed").as_bytes());
            return;
        }
    }

    // P0-2: /v1/* endpoints require Bearer token.
    // /health and /ready are intentionally unauthenticated for liveness probes.
    if path.starts_with("/v1/") {
        let auth = parsed
            .headers
            .iter()
            .find(|(k, _)| k == "authorization")
            .map(|(_, v)| v.as_str())
            .unwrap_or("");
        let expected = format!("Bearer {}", api_token);
        if !constant_time_eq(auth.as_bytes(), expected.as_bytes()) {
            let _ = stream.write_all(json_error(401, "unauthorized", "unauthorized").as_bytes());
            return;
        }
    }

    let response = match (method, path.as_str()) {
        ("GET", "/health") => json_response(200, r#"{"status":"ok"}"#),
        ("GET", "/ready") => json_response(
            200,
            &serde_json::json!({
                "status": "ready",
                "model": model_name
            })
            .to_string(),
        ),
        ("GET", "/v1/models") => {
            let body = serde_json::json!({
                "object": "list",
                "data": [{ "id": model_name, "object": "model" }]
            })
            .to_string();
            json_response(200, &body)
        }
        ("POST", "/v1/chat/completions") => {
            // 流式模式下响应已直接写入 stream，返回 None 跳过统一写响应。
            match handle_chat_completion(&mut stream, &body, &inf, &model_name, origin.as_deref()) {
                Some(resp) => resp,
                None => return,
            }
        }
        ("OPTIONS", _) => cors_preflight_response(origin.as_deref()),
        (_, "/") => json_response(
            200,
            r#"{"message":"桌面AI API server running","endpoints":["/v1/models","/v1/chat/completions"]}"#,
        ),
        _ => json_error(404, "not_found", "not found"),
    };

    // Inject CORS headers for the validated origin. The origin was already
    // checked against the allow-list above, so we reflect it verbatim.
    let response = inject_cors_headers(&response, origin.as_deref());

    let _ = stream.write_all(response.as_bytes());
}

/// 处理 `/v1/chat/completions`。
///
/// 非流式（stream=false）：返回 `Some(response)`，由调用方统一写出。
/// 流式（stream=true）：SSE 响应在生成过程中逐块实时写入 `stream`，
/// 客户端断开连接会立即停止推理；返回 `None` 表示响应已写出。
fn handle_chat_completion(
    stream: &mut TcpStream,
    body: &str,
    inf: &Arc<Mutex<LlamaInference>>,
    model_name: &str,
    origin: Option<&str>,
) -> Option<String> {
    let req: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return Some(json_error(400, "invalid_json", "invalid JSON")),
    };

    let messages = match extract_messages(&req) {
        Some(msgs) => msgs,
        None => {
            return Some(json_error(
                400,
                "missing_messages",
                "missing messages array",
            ))
        }
    };

    let stream_mode = req["stream"].as_bool().unwrap_or(false);

    // Build chatml prompt from messages
    let allowed_roles: &[&str] = &["system", "user", "assistant"];
    let mut prompt = String::new();
    for msg in &messages {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        if !allowed_roles.contains(&role) {
            return Some(json_error(
                400,
                "invalid_role",
                "invalid role; allowed: system, user, assistant",
            ));
        }
        let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
        // P0-3: sanitise ChatML control tokens in user-supplied content to
        // prevent prompt injection (a malicious client could inject
        // <|im_start|>assistant ... <|im_end|> to hijack the response).
        let safe = crate::inference::sanitize_chatml(content.trim());
        prompt.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", role, safe));
    }
    prompt.push_str("<|im_start|>assistant\n");

    if stream_mode {
        stream_sse_response(stream, inf, prompt, origin);
        None
    } else {
        Some(non_stream_response(inf, model_name, prompt))
    }
}

/// 非流式补全：收集完整输出后一次性返回 JSON。
fn non_stream_response(
    inf: &Arc<Mutex<LlamaInference>>,
    model_name: &str,
    prompt: String,
) -> String {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();

    let inf = Arc::clone(inf);
    let stop = stop_flag.clone();
    thread::spawn(move || {
        crate::inference::run_inference(inf, prompt, stop, tx, 2048);
    });

    let mut output = String::new();
    while let Ok(token) = rx.recv() {
        match token {
            StreamToken::Text(t) => output.push_str(&t),
            StreamToken::Error(e) => {
                log::error!("Inference error: {}", e);
                output.push_str("[error: internal error]");
            }
            StreamToken::Done => break,
        }
        if output.len() > 4096 {
            stop_flag.store(true, Ordering::Relaxed);
            break;
        }
    }

    let id = format!("chatcmpl-{}", chrono::Utc::now().timestamp_millis());
    let resp = serde_json::json!({
        "id": id,
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": model_name,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": output},
            "finish_reason": "stop"
        }]
    });
    json_response(200, &resp.to_string())
}

/// 真流式 SSE 补全：token 生成过程中实时写入响应。
///
/// 攒够 50 个字符（或流结束）刷出一个 SSE data 块，降低小包数量；
/// 客户端断开（write 失败）时立即置 stop 标志停止推理，避免浪费算力。
fn stream_sse_response(
    stream: &mut TcpStream,
    inf: &Arc<Mutex<LlamaInference>>,
    prompt: String,
    origin: Option<&str>,
) {
    let cors_origin = origin.unwrap_or("null");
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nAccess-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Methods: {}\r\nAccess-Control-Allow-Headers: {}\r\nConnection: close\r\n\r\n",
        cors_origin, CORS_ALLOWED_METHODS, CORS_ALLOWED_HEADERS
    );
    if stream.write_all(header.as_bytes()).is_err() {
        return; // 客户端已断开，无需启动推理
    }
    // SSE 需要低延迟逐块送达，关闭 Nagle 合并
    let _ = stream.set_nodelay(true);

    let id = format!("chatcmpl-{}", chrono::Utc::now().timestamp_millis());
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();

    let inf = Arc::clone(inf);
    let stop = stop_flag.clone();
    thread::spawn(move || {
        crate::inference::run_inference(inf, prompt, stop, tx, 2048);
    });

    let mut buf = String::new();
    while let Ok(token) = rx.recv() {
        match token {
            StreamToken::Text(t) => buf.push_str(&t),
            StreamToken::Error(e) => {
                log::error!("Inference error: {}", e);
                buf.push_str("[error: internal error]");
            }
            StreamToken::Done => break,
        }
        if buf.chars().count() >= 50 {
            let chunk = sse_chunk(&id, &buf);
            buf.clear();
            if stream.write_all(chunk.as_bytes()).is_err() {
                // 客户端断开：立即停止推理
                stop_flag.store(true, Ordering::Relaxed);
                return;
            }
        }
    }
    if !buf.is_empty() {
        let chunk = sse_chunk(&id, &buf);
        if stream.write_all(chunk.as_bytes()).is_err() {
            stop_flag.store(true, Ordering::Relaxed);
            return;
        }
    }
    let _ = stream.write_all(b"data: [DONE]\n\n");
}

/// 构造一个 OpenAI 兼容的 SSE data 块。
fn sse_chunk(id: &str, text: &str) -> String {
    format!(
        "data: {}\n\n",
        serde_json::json!({
            "id": id,
            "object": "chat.completion.chunk",
            "choices": [{"delta": {"content": text}, "index": 0}]
        })
    )
}

fn extract_messages(req: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
    req["messages"].as_array().cloned()
}

/// 安全加固后的 HTTP 请求解析结果。
#[derive(Debug)]
struct ParsedRequest {
    method: String,
    path: String,
    body: String,
    origin: Option<String>,
    /// 所有请求头（小写键名, 原始值）。
    headers: Vec<(String, String)>,
}

/// 验证字符是否包含控制字符（0x00-0x1F, 0x7F）。
fn contains_control_chars(s: &str) -> bool {
    s.bytes().any(|b| b < 0x20 || b == 0x7F)
}

/// 验证请求头名称是否只含 RFC 7230 token 字符。
fn is_valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| {
            matches!(b,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' |
                b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' |
                b'*' | b'+' | b'-' | b'.' | b'^' | b'_' |
                b'`' | b'|' | b'~'
            )
        })
}

/// 安全加固的 HTTP 请求解析器。对请求行、请求头进行完善的边界检查，
/// 返回 `Result<ParsedRequest, (u16, &'static str)>` 以便调用方直接返回
/// 合适的 HTTP 错误状态码。
fn parse_http_validated(raw: &str) -> Result<ParsedRequest, (u16, &'static str)> {
    let lines: Vec<&str> = raw.split("\r\n").collect();
    if lines.is_empty() {
        return Err((400, "bad request"));
    }

    // ── 请求行解析 ──
    let request_line = lines[0];

    // 检查请求行是否含控制字符
    if contains_control_chars(request_line) {
        return Err((400, "bad request"));
    }

    let parts: Vec<&str> = request_line.splitn(3, ' ').collect();
    if parts.len() != 3 {
        return Err((400, "bad request"));
    }

    let method_str = parts[0];
    let uri = parts[1];
    let version = parts[2];

    // 方法白名单
    if !ALLOWED_METHODS.contains(&method_str) {
        return Err((405, "method not allowed"));
    }

    // URI 长度限制
    if uri.len() > MAX_URI_LEN {
        return Err((414, "URI too long"));
    }

    // HTTP 版本验证
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err((505, "HTTP version not supported"));
    }

    // 提取路径（去除查询字符串）
    let path = uri.split('?').next().unwrap_or("/").to_string();

    // ── 请求头解析 ──
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut origin: Option<String> = None;
    let mut content_length_count: usize = 0;
    let mut header_section_size: usize = 0;

    for line in &lines[1..] {
        if line.is_empty() {
            break; // 空行标志着请求头结束
        }

        // 请求头数量限制
        if headers.len() >= MAX_HEADER_COUNT {
            return Err((431, "too many headers"));
        }

        // 请求头行累计大小限制
        header_section_size += line.len() + 2; // +2 for \r\n
        if header_section_size > MAX_HEADER_SECTION_SIZE {
            return Err((431, "header section too large"));
        }

        // 检查 null 字节（防止头注入）
        if line.contains('\0') {
            return Err((400, "bad request"));
        }

        // 检查控制字符
        if contains_control_chars(line) {
            return Err((400, "bad request"));
        }

        // 分割头名和头值
        let colon_pos = line.find(':').ok_or((400, "bad request"))?;
        let (name, value) = line.split_at(colon_pos);
        let value = &value[1..]; // 跳过 ':'

        // 验证头名称合法字符
        if !is_valid_header_name(name) {
            return Err((400, "bad request"));
        }

        // 头值长度限制
        let trimmed_value = value.trim();
        if trimmed_value.len() > MAX_HEADER_VALUE_LEN {
            return Err((431, "header value too long"));
        }

        let name_lower = name.to_lowercase();

        // Content-Length 重复检测（HTTP 请求走私防护）
        if name_lower == "content-length" {
            content_length_count += 1;
            if content_length_count > 1 {
                return Err((400, "duplicate Content-Length"));
            }
        }

        // 提取 Origin 头
        if name_lower == "origin" {
            origin = Some(trimmed_value.to_string());
        }

        headers.push((name_lower, trimmed_value.to_string()));
    }

    // 提取请求体
    let body = if let Some(pos) = raw.find("\r\n\r\n") {
        raw[pos + 4..].trim().to_string()
    } else {
        String::new()
    };

    Ok(ParsedRequest {
        method: method_str.to_string(),
        path,
        body,
        origin,
        headers,
    })
}

fn json_response(code: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {code} OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {len}\r\n\r\n{body}",
        code = code,
        len = body.len(),
        body = body
    )
}

/// Build a JSON error response with a machine-readable `code` field so
/// clients can branch on error type instead of parsing human text:
/// `{"error": "<message>", "code": "<code>"}`
fn json_error(status: u16, code: &str, message: &str) -> String {
    let body = serde_json::json!({
        "error": message,
        "code": code,
    })
    .to_string();
    json_response(status, &body)
}

/// Build CORS headers for a validated origin and inject them into an existing
/// HTTP response. When `origin` is `Some`, we reflect that specific origin so
/// browsers treat the response as same-origin for the requesting page. When
/// `origin` is `None` (command-line client), no CORS headers are emitted.
fn inject_cors_headers(response: &str, origin: Option<&str>) -> String {
    let Some(origin) = origin else {
        return response.to_string();
    };
    // Insert CORS headers right after the status line.
    if let Some(pos) = response.find("\r\n") {
        let (status_line, rest) = response.split_at(pos);
        format!(
            "{}\r\nAccess-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Methods: {}\r\nAccess-Control-Allow-Headers: {}{}",
            status_line, origin, CORS_ALLOWED_METHODS, CORS_ALLOWED_HEADERS, rest
        )
    } else {
        response.to_string()
    }
}

/// Build a 204 preflight response with strict CORS headers for the given
/// validated origin.
fn cors_preflight_response(origin: Option<&str>) -> String {
    let origin_value = origin.unwrap_or("null");
    format!(
        "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Methods: {}\r\nAccess-Control-Allow-Headers: {}\r\nAccess-Control-Max-Age: {}\r\nConnection: close\r\n\r\n",
        origin_value, CORS_ALLOWED_METHODS, CORS_ALLOWED_HEADERS, CORS_MAX_AGE
    )
}

/// Read a full HTTP request: headers up to `\r\n\r\n` then `Content-Length`
/// bytes of body, honouring MAX_BODY_SIZE. Replaces the old single 64 KiB
/// `read()` that silently truncated large JSON bodies and ignored TCP
/// fragmentation.
///
/// `deadline` bounds the *total* time spent reading (headers + body) so a
/// slow-loris client dribbling 1 byte / 30s cannot hold a connection slot
/// indefinitely despite the per-read timeout.
fn read_http_request(
    stream: &mut TcpStream,
    deadline: std::time::Instant,
) -> Result<Vec<u8>, String> {
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut tmp = [0u8; 4096];
    let mut header_end: Option<usize> = None;

    // Phase 1: read until we locate the end of headers.
    while header_end.is_none() {
        if std::time::Instant::now() >= deadline {
            return Err("请求超时".into());
        }
        // 请求头区域大小限制（安全加固）
        if buf.len() > MAX_HEADER_SECTION_SIZE + 8192 {
            return Err("请求头过大".into());
        }
        let n = stream
            .read(&mut tmp)
            .map_err(|e| format!("读取失败: {}", e))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            header_end = Some(pos);
        }
    }
    let header_end =
        header_end.ok_or_else(|| "请求格式错误，未找到 HTTP 头结束标记".to_string())?;

    // Phase 2: parse Content-Length from the header block.
    let header_str = std::str::from_utf8(&buf[..header_end])
        .map_err(|_| "HTTP 头包含非 UTF-8 字节".to_string())?;
    let content_length = parse_content_length(header_str).unwrap_or(0);
    if content_length > MAX_BODY_SIZE {
        return Err(format!(
            "请求体过大 ({} bytes, 上限 {} bytes)",
            content_length, MAX_BODY_SIZE,
        ));
    }

    // Phase 3: keep reading until the body is complete.
    let needed = header_end + 4 + content_length;
    while buf.len() < needed {
        if std::time::Instant::now() >= deadline {
            return Err("请求超时".into());
        }
        let n = stream
            .read(&mut tmp)
            .map_err(|e| format!("读取失败: {}", e))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    Ok(buf)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn parse_content_length(headers: &str) -> Option<usize> {
    let mut found: Option<usize> = None;
    let mut count = 0usize;
    for line in headers.split("\r\n") {
        // 大小写不敏感匹配
        let lower = line.to_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            count += 1;
            // 重复 Content-Length 检测（HTTP 请求走私防护）
            if count > 1 {
                return None;
            }
            found = rest.trim().parse().ok();
        }
    }
    found
}

/// True if the given `Origin` URL points at an allowed host (localhost or
/// 127.0.0.1) on any port. Properly strips userinfo and path components
/// before comparing the host, preventing `http://localhost:evil@attacker.com`
/// style bypasses.
fn origin_allowed(origin: &str) -> bool {
    for host in &ALLOWED_ORIGIN_HOSTS {
        for scheme in &["http://", "https://"] {
            let rest = match origin.strip_prefix(scheme) {
                Some(r) => r,
                None => continue,
            };
            // 去掉可选的 userinfo（user:pass@host）
            let after_at = rest.rsplit('@').next().unwrap_or(rest);
            // 截断到第一个路径/查询/fragment 分隔符
            let host_port = after_at.split(&['/', '?', '#'][..]).next().unwrap_or("");
            // 处理 IPv6 方括号表示（如 [::1]:8080）
            let host_only = if host_port.starts_with('[') {
                host_port
                    .split(']')
                    .next()
                    .unwrap_or("")
                    .trim_start_matches('[')
            } else {
                // 剥离可选端口（IPv4 或主机名）
                host_port.split(':').next().unwrap_or("")
            };
            if host_only == *host {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_origin_allowed_localhost() {
        assert!(origin_allowed("http://localhost"));
        assert!(origin_allowed("http://localhost:8080"));
        assert!(origin_allowed("http://localhost/app"));
        assert!(origin_allowed("http://127.0.0.1:3000"));
        assert!(origin_allowed("https://127.0.0.1"));
        // IPv6 回环地址
        assert!(origin_allowed("http://[::1]:8080"));
        assert!(origin_allowed("http://[::1]"));
        // 合法 userinfo + 合法 host
        assert!(origin_allowed("http://user:pass@localhost:8080"));
        // 合法 host 带路径
        assert!(origin_allowed("http://localhost:8080/path"));
    }

    #[test]
    fn test_origin_rejects_external() {
        assert!(!origin_allowed("http://evil.com"));
        assert!(!origin_allowed("http://localhost.evil.com"));
        assert!(!origin_allowed("https://attacker.example/localhost"));
        assert!(!origin_allowed("http://192.168.1.1"));
    }

    #[test]
    fn test_origin_rejects_userinfo_bypass() {
        // userinfo 注入绕过：host 实际是 attacker.com
        assert!(!origin_allowed("http://localhost:evil@attacker.com"));
        // 子域名伪装
        assert!(!origin_allowed("http://localhost.evil.com"));
    }

    #[test]
    fn test_constant_time_eq_equal() {
        assert!(constant_time_eq(b"hello", b"hello"));
    }

    #[test]
    fn test_constant_time_eq_different_same_len() {
        assert!(!constant_time_eq(b"hello", b"world"));
    }

    #[test]
    fn test_constant_time_eq_different_len() {
        assert!(!constant_time_eq(b"short", b"much longer"));
    }

    #[test]
    fn test_constant_time_eq_empty() {
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn test_parse_content_length() {
        assert_eq!(
            parse_content_length("GET / HTTP/1.1\r\nContent-Length: 42\r\n"),
            Some(42)
        );
        assert_eq!(
            parse_content_length("GET / HTTP/1.1\r\ncontent-length: 7\r\n"),
            Some(7)
        );
        assert_eq!(parse_content_length("GET / HTTP/1.1\r\n"), None);
    }

    #[test]
    fn test_find_subslice() {
        assert_eq!(find_subslice(b"abc\r\n\r\ndef", b"\r\n\r\n"), Some(3));
        assert_eq!(find_subslice(b"abcdef", b"\r\n\r\n"), None);
    }

    #[test]
    fn test_parse_http_extracts_origin() {
        let raw = "POST /v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: http://localhost:8080\r\nContent-Type: application/json\r\n\r\n{}";
        let parsed = parse_http_validated(raw).expect("有效请求应解析成功");
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.path, "/v1/chat/completions");
        assert_eq!(parsed.body, "{}");
        assert_eq!(parsed.origin.as_deref(), Some("http://localhost:8080"));
    }

    #[test]
    fn test_parse_http_no_origin() {
        let raw = "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let parsed = parse_http_validated(raw).expect("有效请求应解析成功");
        assert!(parsed.origin.is_none());
    }

    #[test]
    fn test_parse_http_rejects_disallowed_method() {
        let raw = "DELETE /v1/models HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let result = parse_http_validated(raw);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().0, 405);
    }

    #[test]
    fn test_parse_http_rejects_bad_version() {
        let raw = "GET / HTTP/2.0\r\nHost: 127.0.0.1\r\n\r\n";
        let result = parse_http_validated(raw);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().0, 505);
    }

    #[test]
    fn test_parse_http_accepts_http10() {
        let raw = "GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n";
        let parsed = parse_http_validated(raw).expect("HTTP/1.0 应被接受");
        assert_eq!(parsed.method, "GET");
    }

    #[test]
    fn test_parse_http_rejects_long_uri() {
        let long_path = "a".repeat(MAX_URI_LEN + 1);
        let raw = format!("GET /{} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n", long_path);
        let result = parse_http_validated(&raw);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().0, 414);
    }

    #[test]
    fn test_parse_http_rejects_null_byte_in_header() {
        let raw = "GET / HTTP/1.1\r\nX-Injected: val\0ue\r\n\r\n";
        let result = parse_http_validated(raw);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().0, 400);
    }

    #[test]
    fn test_parse_http_rejects_duplicate_content_length() {
        let raw = "POST / HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 10\r\n\r\nhello";
        let result = parse_http_validated(raw);
        assert!(result.is_err());
        assert!(result.unwrap_err().1.contains("Content-Length"));
    }

    #[test]
    fn test_parse_http_rejects_control_chars_in_request_line() {
        let raw = "GET /hea\x01lth HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let result = parse_http_validated(raw);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().0, 400);
    }

    #[test]
    fn test_parse_http_rejects_invalid_header_name() {
        let raw = "GET / HTTP/1.1\r\nX-Bad Header: value\r\n\r\n";
        let result = parse_http_validated(raw);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().0, 400);
    }

    #[test]
    fn test_parse_http_rejects_too_many_headers() {
        let mut raw = String::from("GET / HTTP/1.1\r\n");
        for i in 0..MAX_HEADER_COUNT + 1 {
            raw.push_str(&format!("X-Header-{}: value\r\n", i));
        }
        raw.push_str("\r\n");
        let result = parse_http_validated(&raw);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().0, 431);
    }

    #[test]
    fn test_parse_http_rejects_long_header_value() {
        let long_val = "x".repeat(MAX_HEADER_VALUE_LEN + 1);
        let raw = format!("GET / HTTP/1.1\r\nX-Long: {}\r\n\r\n", long_val);
        let result = parse_http_validated(&raw);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().0, 431);
    }

    #[test]
    fn test_parse_content_length_rejects_duplicate() {
        let dup = "POST / HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 10\r\n";
        assert_eq!(parse_content_length(dup), None);
    }

    #[test]
    fn test_sse_chunk_format() {
        // OpenAI 兼容 chunk：`data: {json}\n\n`
        let chunk = sse_chunk("chatcmpl-123", "你好世界");
        assert!(chunk.starts_with("data: "), "chunk: {}", chunk);
        assert!(chunk.ends_with("\n\n"), "chunk: {}", chunk);
        let json_str = chunk.trim_start_matches("data: ").trim_end();
        let v: serde_json::Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(v["object"], "chat.completion.chunk");
        assert_eq!(v["id"], "chatcmpl-123");
        assert_eq!(v["choices"][0]["delta"]["content"], "你好世界");
        assert_eq!(v["choices"][0]["index"], 0);
    }

    #[test]
    fn test_sse_chunk_empty_text() {
        let chunk = sse_chunk("chatcmpl-1", "");
        let json_str = chunk.trim_start_matches("data: ").trim_end();
        let v: serde_json::Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(v["choices"][0]["delta"]["content"], "");
    }

    #[test]
    fn test_constant_time_eq_prefix_share() {
        // 前缀相同、末尾不同的 token 必须判为不相等
        assert!(!constant_time_eq(b"token-abcdef", b"token-ABCDEF"));
        assert!(!constant_time_eq(b"a", b""));
    }

    #[test]
    fn test_constant_time_eq_binary_bytes() {
        // 包含 0x00 与高位字节的输入
        assert!(constant_time_eq(&[0u8, 1, 2], &[0u8, 1, 2]));
        assert!(!constant_time_eq(&[0u8, 1, 2], &[0u8, 1, 3]));
        assert!(!constant_time_eq(&[255u8], &[254u8]));
    }

    #[test]
    fn test_inject_cors_headers_with_origin() {
        let base = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
        let out = inject_cors_headers(base, Some("http://localhost:8080"));
        let head = out.split("\r\n\r\n").next().unwrap();
        assert!(head.starts_with("HTTP/1.1 200 OK"));
        assert!(head.contains("Access-Control-Allow-Origin: http://localhost:8080"));
        assert!(head.contains("Access-Control-Allow-Methods: GET, POST"));
        assert!(head.contains("Access-Control-Allow-Headers: Content-Type, Authorization"));
        // 状态行必须是第一行, CORS 头插在状态行之后
        assert!(out.contains("200 OK\r\nAccess-Control-Allow-Origin:"));
        // body 原样保留
        assert!(out.ends_with("\r\n\r\n{}"));
    }

    #[test]
    fn test_inject_cors_headers_no_origin() {
        let base = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
        assert_eq!(inject_cors_headers(base, None), base);
    }

    #[test]
    fn test_inject_cors_headers_malformed_response() {
        // 无状态行分隔符时原样返回
        let base = "garbage-no-crlf";
        assert_eq!(inject_cors_headers(base, Some("http://localhost")), base);
    }

    #[test]
    fn test_json_error_has_machine_readable_code() {
        let resp = json_error(401, "unauthorized", "unauthorized");
        assert!(resp.starts_with("HTTP/1.1 401 OK"));
        let body = resp.split("\r\n\r\n").nth(1).unwrap();
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(v["code"], "unauthorized");
        assert_eq!(v["error"], "unauthorized");
        assert!(resp.contains(&format!("Content-Length: {}", body.len())));
    }

    #[test]
    fn test_json_error_code_variants() {
        for (status, code) in [
            (400u16, "invalid_json"),
            (403, "origin_not_allowed"),
            (404, "not_found"),
        ] {
            let resp = json_error(status, code, "x");
            assert!(resp.starts_with(&format!("HTTP/1.1 {} OK", status)));
            let body = resp.split("\r\n\r\n").nth(1).unwrap();
            let v: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["code"], code);
        }
    }
}
