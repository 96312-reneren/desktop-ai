//! 无界面启动 OpenAI 兼容 API 服务（自动化 / 中转场景）。
//!
//! 用法:
//!   cargo run --release --example api_serve -- <model.gguf> [port=11434] [token=da-local]
//!
//! 服务只监听 127.0.0.1（与 GUI 内的 API 开关一致）。

use std::sync::{Arc, Mutex};
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: api_serve <model.gguf> [port=11434] [token=da-local]");
        std::process::exit(2);
    }
    let model_path = &args[1];
    let port: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(11434);
    let token = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| "da-local".to_string());
    let model_name = std::path::Path::new(model_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "local-model".to_string());

    let inference = match desktop_ai::LlamaInference::load_ex(model_path, 2048, 4, 0) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("模型加载失败: {}", e);
            std::process::exit(1);
        }
    };
    let inference = Arc::new(Mutex::new(inference));

    let _server = desktop_ai::ApiServer::start(inference, port, model_name, token);
    println!("API 服务已启动: http://127.0.0.1:{}/v1", port);
    println!("按 Ctrl+C 退出");
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
