use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, atomic::AtomicBool};
use std::thread;

use crate::inference::LlamaInference;

pub struct ApiServer {
    stop_flag: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl ApiServer {
    pub fn start(inf: Arc<LlamaInference>, port: u16, active_model: String) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop = stop_flag.clone();

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
            listener.set_nonblocking(true).ok();

            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let inf = Arc::clone(&inf);
                        let model_name = active_model.clone();
                        thread::spawn(move || handle_client(stream, inf, model_name));
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
        self.stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for ApiServer {
    fn drop(&mut self) { self.stop(); }
}

fn handle_client(mut stream: TcpStream, inf: Arc<LlamaInference>, model_name: String) {
    let mut buf = [0u8; 65536];
    let n = match stream.read(&mut buf) {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let request = String::from_utf8_lossy(&buf[..n]);
    let (method, path, body) = parse_http(&request);

    let response = match (method, path.as_str()) {
        ("GET", "/health") => json_response(200, r#"{"status":"ok"}"#),
        ("GET", "/v1/models") => json_response(200, &format!(
            r#"{{"object":"list","data":[{{"id":"{}","object":"model"}}]}}"#,
            model_name
        )),
        ("POST", "/v1/chat/completions") => handle_chat_completion(body, &inf, &model_name),
        ("OPTIONS", _) => cors_response(),
        (_, "/") => json_response(200, r#"{"message":"桌面AI API server running","endpoints":["/v1/models","/v1/chat/completions"]}"#),
        _ => json_response(404, r#"{"error":"not found"}"#),
    };

    let _ = stream.write_all(response.as_bytes());
}

fn handle_chat_completion(body: &str, inf: &Arc<LlamaInference>, model_name: &str) -> String {
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
    let mut prompt = String::new();
    for msg in &messages {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
        prompt.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", role, content));
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
            crate::inference::StreamToken::Text(t) => output.push_str(&t),
            crate::inference::StreamToken::Error(e) => output.push_str(&format!("[error: {}]", e)),
            crate::inference::StreamToken::Done => break,
        }
        if output.len() > 4096 {
            stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);
            break;
        }
    }

    if stream_mode {
        // SSE streaming response
        let id = format!("chatcmpl-{}", chrono::Utc::now().timestamp_millis());
        let mut sse = String::from("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n");
        for chunk in output.chars().collect::<Vec<_>>().chunks(50) {
            let text: String = chunk.iter().collect();
            sse.push_str(&format!("data: {}\n\n", serde_json::json!({
                "id": &id,
                "object": "chat.completion.chunk",
                "choices": [{"delta": {"content": text}, "index": 0}]
            }).to_string()));
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

fn parse_http(raw: &str) -> (&str, String, &str) {
    let lines: Vec<&str> = raw.split("\r\n").collect();
    if lines.is_empty() { return ("GET", "/".into(), ""); }

    let first: Vec<&str> = lines[0].split_whitespace().collect();
    let method = if first.len() > 0 { first[0] } else { "GET" };
    let path = if first.len() > 1 { first[1].to_string() } else { "/".into() };

    // Find body after \r\n\r\n
    let body = if let Some(pos) = raw.find("\r\n\r\n") {
        &raw[pos + 4..]
    } else {
        ""
    };

    (method, path, body.trim())
}

fn json_response(code: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {code} OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\nContent-Length: {len}\r\n\r\n{body}",
        code = code,
        len = body.len(),
        body = body
    )
}

fn cors_response() -> String {
    "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nConnection: close\r\n\r\n".into()
}
