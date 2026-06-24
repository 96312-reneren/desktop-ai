// ─── integration tests: smoke, fuzz, and edge-case verification ───
// Requires: cargo test --test integration
//
// Tests that need a loaded model are guarded by `model_available()` and
// will be skipped with a clear message when the model is absent.

use desktop_ai::config::{self, Config};
use desktop_ai::conversation::Conversation;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

// ─── 1. Edge-case: cleaner fed with random binary / GBK-like bytes ───

#[test]
fn cleaner_handles_random_binary_without_panic() {
    for i in 0..1000 {
        let len = i % 4097 + 1;
        let garbage: String = (0..len)
            .map(|j| ((i as u8).wrapping_add(j as u8)) as char)
            .collect();
        let _ = desktop_ai::cleaner::clean_text(&garbage, "text");
        let _ = desktop_ai::cleaner::clean_text(&garbage, "html");
        let _ = desktop_ai::cleaner::clean_text(&garbage, "markdown");
    }
}

#[test]
fn cleaner_handles_gbk_like_bytes() {
    // Construct a GBK-like byte sequence: GBK 中文你好 = b'\xB0\xA1\xC4\xE3\xBA\xC3'
    let raw = [
        0x47u8, 0x42, 0x4B, 0x20, 0xB0, 0xA1, 0xC4, 0xE3, 0x20, 0xBA, 0xC3, 0x20,
    ];
    let mixed = String::from_utf8_lossy(&raw);
    // Feed this lossy string through the cleaner — must not panic
    let (title, body) = desktop_ai::cleaner::clean_text(&mixed, "html");
    assert!(!title.is_empty() || !body.is_empty());
}

#[test]
fn cleaner_empty_input() {
    assert!(desktop_ai::cleaner::clean_text("", "text").1.is_empty());
    assert!(desktop_ai::cleaner::clean_text("", "html").1.is_empty());
}

// ─── 2. Edge-case: chunker fed with random long strings ───

#[test]
fn chunker_handles_random_long_sized_strings() {
    for size in &[0usize, 1, 50, 200, 500, 2000, 10000] {
        let s: String = (0..*size)
            .map(|i| match i % 5 {
                1 => '。',
                2 => '！',
                3 => '？',
                4 => '\n',
                _ => 'A',
            })
            .collect();
        let chunks = desktop_ai::chunker::chunk_text(&s, 500, 50);
        for c in &chunks {
            let cc = c.chars().count();
            assert!(cc <= 500, "chunk {} chars > 500 (len={})", cc, cc);
        }
    }
}

#[test]
fn chunker_huge_repeated_line() {
    let s = "A".repeat(500_000);
    let chunks = desktop_ai::chunker::chunk_text(&s, 500, 50);
    for c in &chunks {
        assert!(c.chars().count() <= 500);
    }
    assert!(
        chunks.len() >= 950,
        "expected ~1000 chunks, got {}",
        chunks.len()
    );
}

// ─── 3. Concurrent conversation stress test ───────────

#[test]
fn concurrent_conversation_crud_no_race() {
    let done = Arc::new(AtomicBool::new(false));
    let mut handles = vec![];

    for _t in 0..8 {
        let d = Arc::clone(&done);
        handles.push(thread::spawn(move || {
            while !d.load(Ordering::Relaxed) {
                let mut conv = Conversation::new();
                conv.add_message("user", "hello");
                conv.add_message("assistant", "hi");
                let _ = Conversation::load(&conv.id);
                Conversation::delete(&conv.id);
            }
        }));
    }

    thread::sleep(Duration::from_millis(600));
    done.store(true, Ordering::Relaxed);
    for h in handles {
        h.join()
            .expect("thread panicked during concurrent conversation ops");
    }
}

// ─── 4. Config round-trip ─────────────────────────────

#[test]
fn config_save_load_roundtrip() {
    let mut cfg = Config::default();
    cfg.theme = "light".into();
    cfg.font_size = 18;
    cfg.api_enabled = true;
    cfg.api_port = 9999;
    config::save_config(&cfg);

    let loaded = config::load_config();
    assert_eq!(loaded.theme, "light");
    assert_eq!(loaded.font_size, 18);
    assert!(loaded.api_enabled);
    assert_eq!(loaded.api_port, 9999);

    let def = Config::default();
    config::save_config(&def);
}

// ─── 5. API smoke test (requires model) ────────────────

fn model_available() -> bool {
    std::path::Path::new("llama.dll").exists()
        && std::fs::read_dir(config::models_dir())
            .map(|iter| {
                iter.flatten()
                    .any(|e| e.file_name().to_string_lossy().ends_with(".gguf"))
            })
            .unwrap_or(false)
}

#[test]
fn api_server_smoke_test() {
    if !model_available() {
        eprintln!(
            "SKIP api_server_smoke_test: no GGUF model found in {:?}",
            config::models_dir()
        );
        return;
    }

    let mut config = config::load_config();
    let model_id = match &config.selected_model_id {
        Some(id) => id.clone(),
        None => {
            eprintln!("SKIP: no selected_model_id in config");
            return;
        }
    };

    let model_info = match desktop_ai::model_catalog::find_model(&config.model_catalog, &model_id) {
        Some(i) => i.clone(),
        None => {
            eprintln!("SKIP: model not in catalog");
            return;
        }
    };

    let model_path = config::models_dir().join(&model_info.filename);
    if !model_path.exists() {
        eprintln!("SKIP: model file missing: {:?}", model_path);
        return;
    }

    let inf = match desktop_ai::inference::LlamaInference::load_ex(
        &model_path.to_string_lossy(),
        2048,
        4,
        config.gpu_layers,
    ) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("SKIP: model load failed: {}", e);
            return;
        }
    };
    let inference = Arc::new(std::sync::Mutex::new(inf));

    let test_port = 11435u16;
    let mut server = desktop_ai::api_server::ApiServer::start(
        Arc::clone(&inference),
        test_port,
        model_id.clone(),
        "test-token".into(),
    );
    thread::sleep(Duration::from_millis(300));

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("build client");

    // Health
    let resp = client
        .get(format!("http://127.0.0.1:{}/health", test_port))
        .send()
        .expect("health request");
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().expect("parse health json");
    assert_eq!(body["status"], "ok");

    // Ready
    let resp = client
        .get(format!("http://127.0.0.1:{}/ready", test_port))
        .send()
        .expect("ready request");
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().expect("parse ready json");
    assert_eq!(body["status"], "ready");

    // v1/models
    let resp = client
        .get(format!("http://127.0.0.1:{}/v1/models", test_port))
        .send()
        .expect("models request");
    assert_eq!(resp.status().as_u16(), 200);

    server.stop();
}
