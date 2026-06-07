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
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(mut config) = serde_json::from_str::<Config>(&data) {
                if config.model_catalog.is_empty() {
                    config.model_catalog = super::model_catalog::default_catalog();
                }
                return config;
            }
        }
    }
    let config = Config::default();
    save_config(&config);
    config
}

pub fn save_config(config: &Config) {
    let path = config_path();
    if let Ok(data) = serde_json::to_string_pretty(config) {
        std::fs::write(&path, data).ok();
    }
}
