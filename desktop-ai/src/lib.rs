//! # 桌面AI
//!
//! 纯 Rust、完全本地运行的 AI 桌面助手：本地大模型推理 + RAG 知识库 +
//! 网页检索 + OpenAI 兼容 API。
//!
//! 内部实现按业务划分为六个领域模块，均不对外公开：
//!
//! | 领域 | 职责 |
//! |------|------|
//! | `llm` | llama.cpp FFI、推理、向量化、模型目录与下载 |
//! | `rag` | 分块、清洗、爬虫、搜索、本地向量库 |
//! | `store` | 配置、SQLite、对话历史 |
//! | `server` | OpenAI 兼容 API、文件沙盒 |
//! | `ui` | egui 应用与启动引导 |
//! | `platform` | Android 入口、桌面快捷方式 |
//!
//! crate 根只导出「二进制入口」与「集成测试」真正需要的少量条目，
//! 其余实现细节通过 `pub(crate)` 限制在 crate 内部。

// ── 领域模块：仅 crate 内部可见 ──
mod llm;
mod platform;
mod rag;
mod server;
mod store;
mod ui;

// ── crate 内部扁平别名：让各领域模块继续使用 `crate::config`、
//    `crate::ffi` 等路径，避免跨领域循环书写长路径 ──
pub(crate) use llm::{downloader, embedding, ffi, inference, model_catalog};
pub(crate) use platform::shortcut;
pub(crate) use rag::{chunker, cleaner, crawler, search, vector_store};
pub(crate) use server::{api_server, sandbox};
pub(crate) use store::{config, conversation, db};
pub(crate) use ui::markdown;

// ── 对外稳定 API：只暴露二进制入口与集成测试所需条目 ──
pub use llm::ffi::llama_library_name;
pub use llm::inference::LlamaInference;
pub use llm::model_catalog::find_model;
pub use rag::chunker::chunk_text;
pub use rag::cleaner::clean_text;
pub use server::api_server::ApiServer;
pub use store::config::{load_config, models_dir, save_config, Config, ModelInfo, ModelPart};
pub use store::conversation::{Conversation, Message};
pub use ui::startup::run;

/// Android 原生入口所在模块（`android_main` / JNI 符号需对外可达以便导出）。
#[cfg(target_os = "android")]
pub use platform::android;
