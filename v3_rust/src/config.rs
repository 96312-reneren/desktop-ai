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
}

fn default_api_port() -> u16 { 11434 }

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
        }
    }
}

pub fn app_dirs() -> ProjectDirs {
    ProjectDirs::from("com", "desktopai", "DesktopAI")
        .expect("failed to get project directories")
}

pub fn config_path() -> PathBuf {
    let dir = app_dirs().config_dir().to_path_buf();
    std::fs::create_dir_all(&dir).ok();
    dir.join("config.json")
}

pub fn models_dir() -> PathBuf {
    let dir = app_dirs().data_dir().join("models");
    std::fs::create_dir_all(&dir).ok();
    dir
}

pub fn conversations_dir() -> PathBuf {
    let dir = app_dirs().data_dir().join("conversations");
    std::fs::create_dir_all(&dir).ok();
    dir
}

pub fn load_config() -> Config {
    let path = config_path();
    let mut config = if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(mut config) = serde_json::from_str::<Config>(&data) {
                if config.model_catalog.is_empty() {
                    config.model_catalog = super::model_catalog::default_catalog();
                }
                config
            } else {
                Config::default()
            }
        } else {
            Config::default()
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
    config
}

pub fn save_config(config: &Config) {
    let path = config_path();
    if let Ok(data) = serde_json::to_string_pretty(config) {
        std::fs::write(&path, data).ok();
    }
}
