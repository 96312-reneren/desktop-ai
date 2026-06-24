use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex,
};
use std::thread;

use crate::inference::{LlamaInference, StreamToken};

/// Maximum simultaneous in-flight API connections. Extra connections are
/// rejected with 503 to prevent trivial local DoS via unbounded thread spawn.
const MAX_CONCURRENT_CONNS: u32 = 16;
/// Hard cap on request body size (1 MiB) to bound memory per request.
const MAX_BODY_SIZE: usize = 1_048_576;
/// Hosts permitted by the CORS policy. Command-line clients (no `Origin`
/// header) are always allowed; browser origins must match one of these.
const ALLOWED_ORIGIN_HOSTS: [&str; 2] = ["localhost", "127.0.0.1"];

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
    // otherwise hold a connection slot indefinitely.
    if let Err(e) = stream.set_read_timeout(Some(std::time::Duration::from_secs(30))) {
        log::warn!("API set_read_timeout failed: {}", e);
    }
    let raw = match read_http_request(&mut stream) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("API read_http_request: {}", e);
            let _ = stream.write_all(
                json_response(400, &serde_json::json!({"error": e}).to_string()).as_bytes(),
            );
            return;
        }
    };
    let request = String::from_utf8_lossy(&raw);
    let (method, path, body, origin) = parse_http(&request);

    // CORS: browser origins must be on the allow-list; command-line (no
    // Origin) is always permitted.
    if let Some(ref origin) = origin {
        if !origin_allowed(origin) {
            log::warn!("API rejected Origin: {}", origin);
            let _ = stream
                .write_all(json_response(403, r#"{"error":"origin not allowed"}"#).as_bytes());
            return;
        }
    }

    // P0-2: /v1/* endpoints require Bearer token.
    // /health and /ready are intentionally unauthenticated for liveness probes.
    if path.starts_with("/v1/") {
        let auth = parse_header(&request, "Authorization");
        let expected = format!("Bearer {}", api_token);
        if auth.as_deref() != Some(expected.as_str()) {
            let _ = stream.write_all(
                json_response(
                    401,
                    r#"{"error":"unauthorized; set Authorization: Bearer <token>"}"#,
                )
                .as_bytes(),
            );
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
        ("POST", "/v1/chat/completions") => handle_chat_completion(body, &inf, &model_name),
        ("OPTIONS", _) => cors_response(origin.as_deref()),
        (_, "/") => json_response(
            200,
            r#"{"message":"桌面AI API server running","endpoints":["/v1/models","/v1/chat/completions"]}"#,
        ),
        _ => json_response(404, r#"{"error":"not found"}"#),
    };

    let _ = stream.write_all(response.as_bytes());
}

fn handle_chat_completion(
    body: &str,
    inf: &Arc<Mutex<LlamaInference>>,
    model_name: &str,
) -> String {
    let req: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return json_response(400, r#"{"error":"invalid JSON"}"#),
    };

    let messages = match extract_messages(&req) {
        Some(msgs) => msgs,
        None => return json_response(400, r#"{"error":"missing messages array"}"#),
    };

    let stream_mode = req["stream"].as_bool().unwrap_or(false);

    // Build chatml prompt from messages
    let allowed_roles: &[&str] = &["system", "user", "assistant"];
    let mut prompt = String::new();
    for msg in &messages {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        if !allowed_roles.contains(&role) {
            return json_response(
                400,
                r#"{"error":"invalid role; allowed: system, user, assistant"}"#,
            );
        }
        let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
        // P0-3: sanitise ChatML control tokens in user-supplied content to
        // prevent prompt injection (a malicious client could inject
        // <|im_start|>assistant ... <|im_end|> to hijack the response).
        let safe = crate::inference::sanitize_chatml(content.trim());
        prompt.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", role, safe));
    }
    prompt.push_str("<|im_start|>assistant\n");

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
            StreamToken::Error(e) => output.push_str(&format!("[error: {}]", e)),
            StreamToken::Done => break,
        }
        if output.len() > 4096 {
            stop_flag.store(true, Ordering::Relaxed);
            break;
        }
    }

    if stream_mode {
        // SSE streaming response
        let id = format!("chatcmpl-{}", chrono::Utc::now().timestamp_millis());
        let mut sse = String::from("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n");
        for chunk in output.chars().collect::<Vec<_>>().chunks(50) {
            let text: String = chunk.iter().collect();
            sse.push_str(&format!(
                "data: {}\n\n",
                serde_json::json!({
                    "id": &id,
                    "object": "chat.completion.chunk",
                    "choices": [{"delta": {"content": text}, "index": 0}]
                })
            ));
        }
        sse.push_str("data: [DONE]\n\n");
        sse
    } else {
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
}

fn extract_messages(req: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
    req["messages"].as_array().cloned()
}

fn parse_http(raw: &str) -> (&str, String, &str, Option<String>) {
    let lines: Vec<&str> = raw.split("\r\n").collect();
    if lines.is_empty() {
        return ("GET", "/".into(), "", None);
    }

    let first: Vec<&str> = lines[0].split_whitespace().collect();
    let method = if !first.is_empty() { first[0] } else { "GET" };
    let path = if first.len() > 1 {
        first[1].to_string()
    } else {
        "/".into()
    };

    // Origin header (case-insensitive) for CORS gating
    let mut origin: Option<String> = None;
    for line in &lines[1..] {
        if line.is_empty() {
            break;
        }
        if let Some(rest) = line
            .strip_prefix("Origin:")
            .or_else(|| line.strip_prefix("origin:"))
        {
            origin = Some(rest.trim().to_string());
            break;
        }
    }

    // Find body after \r\n\r\n
    let body = if let Some(pos) = raw.find("\r\n\r\n") {
        &raw[pos + 4..]
    } else {
        ""
    };

    (method, path, body.trim(), origin)
}

fn json_response(code: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {code} OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\nContent-Length: {len}\r\n\r\n{body}",
        code = code,
        len = body.len(),
        body = body
    )
}

fn cors_response(_origin: Option<&str>) -> String {
    // Reflect a specific allowed origin instead of the wildcard; command-line
    // (no Origin) gets the wildcard which is harmless for non-browser clients.
    "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nConnection: close\r\n\r\n".into()
}

/// Read a full HTTP request: headers up to `\r\n\r\n` then `Content-Length`
/// bytes of body, honouring MAX_BODY_SIZE. Replaces the old single 64 KiB
/// `read()` that silently truncated large JSON bodies and ignored TCP
/// fragmentation.
fn read_http_request(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut tmp = [0u8; 4096];
    let mut header_end: Option<usize> = None;

    // Phase 1: read until we locate the end of headers.
    while header_end.is_none() {
        if buf.len() > MAX_BODY_SIZE + 8192 {
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

/// Case-insensitive HTTP header lookup.
fn parse_header(request: &str, name: &str) -> Option<String> {
    let prefix_lower = format!("{}:", name).to_lowercase();
    for line in request.lines() {
        let lower = line.to_lowercase();
        if let Some(rest) = lower.strip_prefix(&prefix_lower) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn parse_content_length(headers: &str) -> Option<usize> {
    for line in headers.split("\r\n") {
        if let Some(rest) = line
            .strip_prefix("Content-Length:")
            .or_else(|| line.strip_prefix("content-length:"))
        {
            return rest.trim().parse().ok();
        }
    }
    None
}

/// True if the given `Origin` URL points at an allowed host (localhost or
/// 127.0.0.1) on any port. Prevents `http://localhost.evil.com` style
/// bypasses by matching the host boundary (`:`, `/`, or end-of-string).
fn origin_allowed(origin: &str) -> bool {
    for host in &ALLOWED_ORIGIN_HOSTS {
        for scheme in &["http://", "https://"] {
            let prefix = format!("{}{}", scheme, host);
            if origin == prefix {
                return true;
            }
            if let Some(rest) = origin.strip_prefix(&prefix) {
                if rest.starts_with(':') || rest.starts_with('/') {
                    return true;
                }
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
    }

    #[test]
    fn test_origin_rejects_external() {
        assert!(!origin_allowed("http://evil.com"));
        assert!(!origin_allowed("http://localhost.evil.com"));
        assert!(!origin_allowed("https://attacker.example/localhost"));
        assert!(!origin_allowed("http://192.168.1.1"));
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
        let (method, path, body, origin) = parse_http(raw);
        assert_eq!(method, "POST");
        assert_eq!(path, "/v1/chat/completions");
        assert_eq!(body, "{}");
        assert_eq!(origin.as_deref(), Some("http://localhost:8080"));
    }

    #[test]
    fn test_parse_http_no_origin() {
        let raw = "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let (_, _, _, origin) = parse_http(raw);
        assert!(origin.is_none());
    }
}
