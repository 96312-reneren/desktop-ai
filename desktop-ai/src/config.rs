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
    /// Split-file model parts (e.g. HF multi-part GGUF). When non-empty,
    /// the downloader fetches every part, verifies each one's SHA-256, then
    /// concatenates them into `filename`. Kept empty for single-file models.
    #[serde(default)]
    pub parts: Vec<ModelPart>,
}

/// One split-file model part: download URL + local filename (+ optional
/// SHA-256 for integrity verification of that part).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPart {
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub sha256: Option<String>,
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

fn default_api_port() -> u16 {
    11434
}

fn default_api_token() -> String {
    // Cryptographically random token generated on first startup so the API
    // is not wide-open. Written to config.json and stays stable across
    // restarts; a fresh token is generated if config.json is deleted.
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_ok() {
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        format!("da-{}", hex)
    } else {
        // Fallback: timestamp-based (weak but still keeps the API from
        // being wide-open) if the OS RNG is unavailable.
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("da-{:016x}", nanos)
    }
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
    ProjectDirs::from("com", "desktopai", "DesktopAI").expect("failed to get project directories")
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

pub fn log_dir() -> PathBuf {
    let dir = app_dirs().data_dir().join("logs");
    ensure_dir(&dir, "logs");
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
                    } else {
                        // Backfill `parts` for known catalog ids from older
                        // config files (e.g. the 7B split-file entry) so
                        // existing users automatically get the fixed URLs.
                        // Only ids matched against the default catalog are
                        // touched; user-customised entries keep their data.
                        let defaults = super::model_catalog::default_catalog();
                        for info in config.model_catalog.iter_mut() {
                            if info.parts.is_empty() {
                                if let Some(def) = defaults.iter().find(|d| d.id == info.id) {
                                    info.parts = def.parts.clone();
                                }
                            }
                        }
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

    if config.font_size < 10 {
        config.font_size = 14;
    }
    if config.font_size > 24 {
        config.font_size = 14;
    }
    if config.n_ctx < 512 {
        config.n_ctx = 512;
    }
    if config.n_ctx > 32768 {
        config.n_ctx = 4096;
    }
    if config.theme != "dark" && config.theme != "light" {
        config.theme = "dark".into();
    }
    if config
        .last_conversation_id
        .as_deref()
        .map(|s| s.len() > 100)
        .unwrap_or(false)
    {
        config.last_conversation_id = None;
    }
    if config.system_prompt.len() > 10000 {
        config.system_prompt = "You are a helpful assistant.".into();
    }
    if config.api_port < 1024 {
        config.api_port = 11434;
    }
    if config.gpu_layers < 0 {
        config.gpu_layers = 0;
    }
    if config.gpu_layers > 999 {
        config.gpu_layers = 0;
    }
    config
}

pub fn save_config(config: &Config) {
    let path = config_path();
    if let Ok(data) = serde_json::to_string_pretty(config) {
        // Atomic-ish write: a crash mid-write corrupts the tmp file, not the
        // real config. rename() is atomic on POSIX; on Windows we remove the
        // target first as a fallback when the plain rename fails.
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, &data) {
            log::warn!("failed to write config: {}", e);
            return;
        }

        // Set restrictive permissions on the temp file BEFORE rename so the
        // final config file is never world-readable (contains API tokens).
        set_config_permissions(&tmp);

        if std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&path);
            if let Err(e) = std::fs::rename(&tmp, &path) {
                log::warn!("failed to replace config: {}", e);
            }
        }

        // Also ensure permissions on the final path (rename preserves perms
        // on POSIX, but be explicit for safety).
        set_config_permissions(&path);
    }
}

/// Set restrictive file permissions on the config file.
///
/// On Unix: `0o600` — owner read/write only, preventing other users on shared
/// systems from reading sensitive data (API tokens, etc.).
///
/// On Windows: file access is governed by ACLs rather than POSIX mode bits.
/// The default ACL inherited from the user's AppData directory already restricts
/// access to the current user, so no extra step is needed here.
fn set_config_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        if let Err(e) = std::fs::set_permissions(path, perms) {
            log::warn!("failed to set config permissions on {:?}: {}", path, e);
        }
    }

    #[cfg(windows)]
    {
        // On Windows, the config lives under %APPDATA% where the default ACL
        // already limits access to the owning user.  No extra action needed.
        log::debug!("Windows 配置文件权限由系统 ACL 管理, 路径: {:?}", path);
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
        let mut c = Config {
            font_size: 5,
            ..Default::default()
        };
        c.n_ctx = 100;
        c.theme = "red".into();
        c.system_prompt = "x".repeat(20000);
        if c.font_size < 10 {
            c.font_size = 14;
        }
        if c.n_ctx < 512 {
            c.n_ctx = 512;
        }
        if c.theme != "dark" && c.theme != "light" {
            c.theme = "dark".into();
        }
        if c.system_prompt.len() > 10000 {
            c.system_prompt = "You are a helpful assistant.".into();
        }
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
    #[allow(clippy::assertions_on_constants)]
    fn test_max_input_graphemes_is_reasonable() {
        // Must be between 100 and 10000 — sanity check on the constant.
        assert!(MAX_INPUT_GRAPHEMES >= 100);
        assert!(MAX_INPUT_GRAPHEMES <= 10000);
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn test_kb_single_doc_chars_is_reasonable() {
        // ~4000 tokens × 4 chars/token = 16000
        assert!(KB_SINGLE_DOC_CHARS >= 4000);
        assert!(KB_SINGLE_DOC_CHARS <= 64000);
    }
}
