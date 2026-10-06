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

// ── 对外稳定 API：只暴露二进制入口与集成测试所需条目 ──
pub use llm::ffi::llama_library_name;
pub use llm::inference::LlamaInference;
pub use llm::model_catalog::find_model;
pub use rag::chunker::chunk_text;
pub use rag::cleaner::clean_text;
pub use server::api_server::ApiServer;
pub use store::config::{load_config, models_dir, save_config, Config, ModelInfo, ModelPart};
pub use store::conversation::{Conversation, Message};
#[cfg(not(target_os = "android"))]
pub use ui::startup::run;

/// Android 原生入口所在模块（`android_main` / JNI 符号需对外可达以便导出）。
#[cfg(target_os = "android")]
pub use platform::android;

/// 红队对抗性用例集：针对 SSRF 黑名单覆盖度、文件沙盒路径穿越、
/// 知识库全文检索注入与语法健壮性。仅测试构建编译。
#[cfg(test)]
mod redteam;

/// 测试专用工具：把数据根目录指向 `target/test-data/unit`，确保任何单元测试
/// 都不会读写真实用户数据（%APPDATA% / ~/.local/share）。目录放在构建输出
/// 内，每次测试运行前整体重建，`cargo clean` 一并清除，不会在系统临时目录
/// 或用户目录留下任何残留。
#[cfg(test)]
pub(crate) mod test_util {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    pub(crate) fn temp_data_dir() -> &'static Path {
        static DIR: OnceLock<PathBuf> = OnceLock::new();
        DIR.get_or_init(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("test-data")
                .join("unit");
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("create unit test data dir");
            std::env::set_var("DESKTOP_AI_DATA_DIR", &root);
            root
        })
        .as_path()
    }
}
