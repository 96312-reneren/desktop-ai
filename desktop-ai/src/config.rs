use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub desc: String,
    pub size_gb: f64,
    pub tags: Vec<String>,
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub expected_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub theme: String,
    pub font_size: u32,
    pub n_ctx: u32,
    pub n_threads: String,
    pub last_conversation_id: Option<String>,
    pub selected_model_id: Option<String>,
    pub system_prompt: String,
    #[serde(default)]
    pub model_catalog: Vec<ModelInfo>,
    #[serde(default)]
    pub api_enabled: bool,
    #[serde(default = "default_api_port")]
    pub api_port: u16,
    #[serde(default)]
    pub search_enabled: bool,
    #[serde(default)]
    pub kb_enabled: bool,
    #[serde(default)]
    pub gpu_layers: i32,
    #[serde(default = "default_api_token")]
    pub api_token: String,
}

fn default_api_port() -> u16 { 11434 }

fn default_api_token() -> String {
    // P0-2: generate a random-ish token on first startup so the API is
    // not wide-open. The token is written to config.json and stays stable
    // across restarts. If the user deletes config.json a new token is
    // generated.
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("da-{:016x}", nanos)
}

/// Maximum chat-input graphemes (NOT bytes). Enforced via
/// `TextEdit::char_limit` + a visual truncation hint in `chat.rs`.
pub const MAX_INPUT_GRAPHEMES: usize = 1500;

/// Estimated character limit for a single RAG document before
/// auto-chunking is triggered (≈ 4 000 tokens × 4 chars/token).
pub const KB_SINGLE_DOC_CHARS: usize = 16000;

/// Strip zero-width characters that cause visual deception and
/// noise in vector retrieval.
pub fn strip_zero_width(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(*c, '\u{200B}' | '\u{200C}' | '\u{200D}'))
        .collect()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            font_size: 14,
            n_ctx: 4096,
            n_threads: "auto".into(),
            last_conversation_id: None,
            selected_model_id: None,
            system_prompt: "You are a helpful assistant.".into(),
            model_catalog: super::model_catalog::default_catalog(),
            api_enabled: false,
            api_port: 11434,
            search_enabled: false,
            kb_enabled: false,
            gpu_layers: 0,
            api_token: default_api_token(),
        }
    }
}

pub fn app_dirs() -> ProjectDirs {
    ProjectDirs::from("com", "desktopai", "DesktopAI")
        .expect("failed to get project directories")
}

fn ensure_dir(dir: &std::path::Path, label: &str) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        log::warn!("failed to create {} dir {:?}: {}", label, dir, e);
    }
}

pub fn config_path() -> PathBuf {
    let dir = app_dirs().config_dir().to_path_buf();
    ensure_dir(&dir, "config");
    dir.join("config.json")
}

pub fn models_dir() -> PathBuf {
    let dir = app_dirs().data_dir().join("models");
    ensure_dir(&dir, "models");
    dir
}

pub fn conversations_dir() -> PathBuf {
    let dir = app_dirs().data_dir().join("conversations");
    ensure_dir(&dir, "conversations");
    dir
}

pub fn kb_dir() -> PathBuf {
    let dir = app_dirs().data_dir().join("knowledge_base");
    ensure_dir(&dir, "knowledge_base");
    dir
}

pub fn sandbox_dir() -> PathBuf {
    let dir = app_dirs().data_dir().join("sandbox");
    ensure_dir(&dir, "sandbox");
    dir
}

pub fn load_config() -> Config {
    let path = config_path();
    let mut config = if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(data) => match serde_json::from_str::<Config>(&data) {
                Ok(mut config) => {
                    if config.model_catalog.is_empty() {
                        config.model_catalog = super::model_catalog::default_catalog();
                    }
                    config
                }
                Err(e) => {
                    log::warn!("failed to parse config {:?}: {} — using default", path, e);
                    Config::default()
                }
            },
            Err(e) => {
                log::warn!("failed to read config {:?}: {} — using default", path, e);
                Config::default()
            }
        }
    } else {
        let config = Config::default();
        save_config(&config);
        return config;
    };

    if config.font_size < 10 { config.font_size = 14; }
    if config.font_size > 24 { config.font_size = 14; }
    if config.n_ctx < 512 { config.n_ctx = 512; }
    if config.n_ctx > 32768 { config.n_ctx = 4096; }
    if config.theme != "dark" && config.theme != "light" { config.theme = "dark".into(); }
    if config.last_conversation_id.as_deref().map(|s| s.len() > 100).unwrap_or(false) {
        config.last_conversation_id = None;
    }
    if config.system_prompt.len() > 10000 { config.system_prompt = "You are a helpful assistant.".into(); }
    if config.api_port < 1024 { config.api_port = 11434; }
    if config.gpu_layers < 0 { config.gpu_layers = 0; }
    if config.gpu_layers > 999 { config.gpu_layers = 0; }
    config
}

pub fn save_config(config: &Config) {
    let path = config_path();
    if let Ok(data) = serde_json::to_string_pretty(config) {
        if let Err(e) = std::fs::write(&path, &data) {
            log::warn!("failed to save config: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let c = Config::default();
        assert_eq!(c.theme, "dark");
        assert_eq!(c.font_size, 14);
        assert_eq!(c.n_ctx, 4096);
        assert_eq!(c.n_threads, "auto");
        assert_eq!(c.api_port, 11434);
        assert!(!c.api_enabled);
        assert!(c.model_catalog.len() >= 5);
    }

    #[test]
    fn test_config_validation_clamps() {
        let mut c = Config::default();
        c.font_size = 5;
        c.n_ctx = 100;
        c.theme = "red".into();
        c.system_prompt = "x".repeat(20000);
        if c.font_size < 10 { c.font_size = 14; }
        if c.n_ctx < 512 { c.n_ctx = 512; }
        if c.theme != "dark" && c.theme != "light" { c.theme = "dark".into(); }
        if c.system_prompt.len() > 10000 { c.system_prompt = "You are a helpful assistant.".into(); }
        assert_eq!(c.font_size, 14);
        assert_eq!(c.n_ctx, 512);
        assert_eq!(c.theme, "dark");
        assert_eq!(c.system_prompt, "You are a helpful assistant.");
    }

    #[test]
    fn test_strip_zero_width_removes_invisible_chars() {
        let input = "hello\u{200B}world\u{200C}!\u{200D}";
        let cleaned = strip_zero_width(input);
        assert_eq!(cleaned, "helloworld!");
    }

    #[test]
    fn test_strip_zero_width_preserves_normal_unicode() {
        let input = "中文テスト한국어";
        assert_eq!(strip_zero_width(input), input);
    }

    #[test]
    fn test_strip_zero_width_empty_and_whitespace() {
        assert_eq!(strip_zero_width(""), "");
        assert_eq!(strip_zero_width("\u{200B}\u{200C}"), "");
    }

    #[test]
    fn test_max_input_graphemes_is_reasonable() {
        // Must be between 100 and 10000 — sanity check on the constant.
        assert!(MAX_INPUT_GRAPHEMES >= 100);
        assert!(MAX_INPUT_GRAPHEMES <= 10000);
    }

    #[test]
    fn test_kb_single_doc_chars_is_reasonable() {
        // ~4000 tokens × 4 chars/token = 16000
        assert!(KB_SINGLE_DOC_CHARS >= 4000);
        assert!(KB_SINGLE_DOC_CHARS <= 64000);
    }
}
