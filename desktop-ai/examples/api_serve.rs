//! 无界面启动 OpenAI 兼容 API 服务（自动化 / 中转场景）。
//!
//! 用法:
//!   cargo run --release --example api_serve -- <model.gguf> [port=11434] [token]
//!
//! token 缺省时自动生成随机值并打印（不再使用弱默认值）。
//! 服务只监听 127.0.0.1（与 GUI 内的 API 开关一致）。

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Cryptographically random token, matching the GUI's scheme.
fn random_token() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_ok() {
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        format!("da-{}", hex)
    } else {
        // Extremely unlikely fallback; still far better than a fixed string.
        format!(
            "da-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        )
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: api_serve <model.gguf> [port=11434] [token]");
        std::process::exit(2);
    }
    let model_path = &args[1];
    let port: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(11434);
    let (token, generated) = match args.get(3) {
        Some(t) => (t.clone(), false),
        None => (random_token(), true),
    };
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

    let _server = desktop_ai::ApiServer::start(inference, port, model_name, token.clone());
    println!("API 服务已启动: http://127.0.0.1:{}/v1", port);
    if generated {
        println!("已自动生成 API token（请配置到客户端）: {}", token);
    }
    println!("按 Ctrl+C 退出");
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
