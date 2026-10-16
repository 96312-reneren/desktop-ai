// DesktopAI application state: data structures, startup checks, constructor.
//
// This module owns the `DesktopAI` struct definition and its initialisation
// logic. Rendering and event-handling live in sibling modules.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use crate::llm::downloader::DownloadMsg;
use crate::llm::inference::{LlamaInference, StreamToken};
use crate::platform::hardware::{detect_gpus, detect_hardware, GpuInfo};
use crate::rag::kb_job::KbIndexMsg;
use crate::rag::vector_store::VectorStore;
use crate::server::api_server::ApiServer;
use crate::server::sandbox::Sandbox;
use crate::store::config::{self, Config};
use crate::store::conversation::Conversation;
use eframe::egui;

pub(crate) fn apply_theme(ctx: &egui::Context, theme: &str) {
    super::theme::apply(ctx, theme == "dark");
}

/// Default number of recent messages rendered in the chat window.
/// Older messages are hidden behind a "load earlier" button to keep
/// the per-frame widget count bounded on long conversations.
pub(crate) const DEFAULT_VISIBLE_MSGS: usize = 50;

/// How many more messages to reveal each time the user clicks
/// "load earlier messages".
pub(crate) const MSG_PAGE_SIZE: usize = 30;

// ─── Download state ────────────────────────────────────

pub(crate) struct DownloadState {
    pub(crate) progress: f32,
    pub(crate) status: String,
    pub(crate) rx: mpsc::Receiver<DownloadMsg>,
    pub(crate) cancel: Arc<AtomicBool>,
}

// ─── Generation state ──────────────────────────────────

pub(crate) struct GenState {
    pub(crate) conv_id: String,
    pub(crate) pending_text: String,
    /// Budget notice shown above the reply (context full / truncated, ...).
    pub(crate) notice: Option<String>,
    /// Knowledge-base references for this reply ("参考来源" section).
    pub(crate) sources: Vec<crate::rag::vector_store::SourceRef>,
    pub(crate) rx: mpsc::Receiver<StreamToken>,
    pub(crate) stop_flag: Arc<AtomicBool>,
}

// ─── KB index state (background indexing) ──────────────

/// 后台知识库索引任务状态：UI 线程轮询 `rx`，`cancel` 可随时中止。
pub(crate) struct KbJobState {
    pub(crate) rx: mpsc::Receiver<KbIndexMsg>,
    pub(crate) cancel: Arc<AtomicBool>,
}

// ─── Model load state (off-UI-thread loading) ──────────

pub(crate) enum ModelLoadResult {
    Loaded {
        inference: LlamaInference,
        embedding: Option<crate::llm::embedding::EmbeddingEngine>,
        model_name: String,
        gpu_tag: String,
    },
    Error(String),
}

pub(crate) struct ModelLoadState {
    pub(crate) rx: mpsc::Receiver<ModelLoadResult>,
}

// ─── Startup diagnostics ───────────────────────────────

/// Startup diagnostics surfaced to the user once, in priority order:
/// missing dependency > crash recovery > missing model > GPU unavailable.
/// Only ONE notice is shown per launch so popups never stack.
#[derive(Debug, Clone)]
pub(crate) enum StartupNotice {
    /// llama 依赖库缺失(致命,无法加载任何模型)
    MissingDependency(String),
    /// 上次异常退出,对话已自动保存
    CrashRecovery,
    /// 配置中选中的模型文件不存在
    ModelMissing(String),
    /// 配置了 GPU 加速但当前 llama 库为 CPU 版
    GpuUnavailable,
}

/// Run the startup self-check. Returns the single highest-priority notice
/// to display (or None when everything is fine).
fn startup_notice(config: &Config) -> Option<StartupNotice> {
    // 1. Runtime dependency (llama library) — fatal.
    let lib_name = crate::llm::ffi::llama_library_name();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();
    let lib_ok = [std::path::PathBuf::from(lib_name), exe_dir.join(lib_name)]
        .iter()
        .any(|p| {
            p.exists()
                && std::fs::metadata(p)
                    .map(|m| m.len() >= 1_000_000)
                    .unwrap_or(false)
        });
    if !lib_ok {
        return Some(StartupNotice::MissingDependency(lib_name.to_string()));
    }

    // 2. Crash recovery (after deps so a broken install doesn't also nag).
    if crate::store::config::should_show_crash_notice() {
        return Some(StartupNotice::CrashRecovery);
    }

    // 3. Selected model file missing.
    if let Some(id) = &config.selected_model_id {
        if let Some(info) = crate::llm::model_catalog::find_model(&config.model_catalog, id) {
            if !crate::store::config::models_dir()
                .join(&info.filename)
                .exists()
            {
                return Some(StartupNotice::ModelMissing(info.name.clone()));
            }
        }
    }

    // 4. GPU configured but the library has no BLAS backend.
    if config.gpu_layers > 0 && !crate::llm::ffi::gpu_backend_available() {
        return Some(StartupNotice::GpuUnavailable);
    }

    None
}

// ─── Confirm actions ───────────────────────────────────

#[derive(Clone, PartialEq)]
pub(crate) enum ConfirmAction {
    DeleteAllModels,
    DeleteAllConversations,
    ResetApp,
    UninstallApp,
}

// ─── App state ─────────────────────────────────────────

pub(crate) struct DesktopAI {
    pub(crate) config: Config,
    pub(crate) inference: Option<Arc<Mutex<LlamaInference>>>,
    pub(crate) current_conv: Conversation,
    /// Startup diagnostics popup (shown once, highest priority only).
    pub(crate) startup_notice: Option<StartupNotice>,
    /// First-run "create desktop shortcut?" prompt (shown once).
    pub(crate) show_shortcut_prompt: bool,

    // Chat
    pub(crate) input_text: String,
    pub(crate) gen: Option<GenState>,
    /// Retrieval references of the most recent reply, shown under the chat
    /// ("参考来源"): (conversation id, sources).
    pub(crate) last_sources: Option<(String, Vec<crate::rag::vector_store::SourceRef>)>,
    /// Thread handle for the active inference. Joined in poll_generation
    /// when Done is received (returns immediately — the thread is already
    /// finished at that point). On abrupt process exit the OS reclaims all
    /// resources; a desktop application does not need a graceful shutdown.
    pub(crate) gen_handle: Option<thread::JoinHandle<()>>,
    pub(crate) model_load: Option<ModelLoadState>,

    // Downloads (multiple concurrent)
    pub(crate) downloads: HashMap<String, DownloadState>,

    // Hardware info
    pub(crate) cpu_cores: usize,
    pub(crate) ram_warning: Option<String>,
    pub(crate) gpu_info: Vec<GpuInfo>,

    // API server
    pub(crate) api_server: Option<ApiServer>,

    // Knowledge base
    pub(crate) vector_store: Arc<VectorStore>,
    pub(crate) sandbox: Sandbox,
    pub(crate) show_kb_panel: bool,
    pub(crate) kb_title: String,
    pub(crate) kb_content: String,
    pub(crate) kb_url: String,
    pub(crate) kb_crawl_depth: u32,
    pub(crate) kb_indexing: bool,
    pub(crate) kb_index_progress: f32,
    pub(crate) kb_index_status: String,
    pub(crate) kb_job: Option<KbJobState>,
    // Keyword search (FTS5)
    pub(crate) kb_search_query: String,
    pub(crate) kb_search_results: Vec<crate::rag::vector_store::SearchHit>,
    pub(crate) kb_search_done: bool,

    // Search
    pub(crate) show_search_panel: bool,
    pub(crate) search_query: String,
    pub(crate) search_results: Vec<crate::rag::search::SearchResult>,
    pub(crate) search_loading: bool,
    pub(crate) search_error: Option<String>,
    pub(crate) search_rx:
        Option<mpsc::Receiver<Result<Vec<crate::rag::search::SearchResult>, String>>>,
    pub(crate) conv_filter: String,

    // UI
    /// How many recent messages to render in the chat area (virtual window).
    /// Older messages are hidden behind a "load earlier" button. Reset to
    /// `DEFAULT_VISIBLE_MSGS` on conversation switch.
    pub(crate) visible_msg_count: usize,
    pub(crate) show_model_select: bool,
    pub(crate) show_settings: bool,
    pub(crate) status_message: String,
    pub(crate) error_message: Option<String>,
    pub(crate) theme_applied: bool,
    pub(crate) confirm_action: Option<ConfirmAction>,
    /// Conversation list cache (sidebar). Refreshed only when dirty.
    pub(crate) conv_cache: Vec<crate::store::conversation::ConversationMeta>,
    pub(crate) conv_cache_dirty: bool,
    /// Display name of the currently loaded model (None = not loaded).
    pub(crate) loaded_model_name: Option<String>,
}

#[allow(clippy::new_without_default)]
impl DesktopAI {
    pub(crate) fn new() -> Self {
        let config = config::load_config();
        let current_conv = if let Some(ref id) = config.last_conversation_id {
            Conversation::load(id).unwrap_or_else(Conversation::new)
        } else {
            Conversation::new()
        };

        let (cpu_cores, ram_warning) = detect_hardware();
        let gpu_info = detect_gpus();
        let vector_store = Arc::new(VectorStore::new(&config::kb_dir()));
        let sandbox = Sandbox::new(config::sandbox_dir());
        let startup_notice = startup_notice(&config);
        // First-run desktop shortcut prompt (only when one doesn't exist yet
        // and the user hasn't answered before).
        let show_shortcut_prompt =
            !config.shortcut_prompted && !crate::platform::shortcut::shortcut_exists();

        let mut app = Self {
            config,
            inference: None,
            current_conv,
            startup_notice,
            show_shortcut_prompt,
            input_text: String::new(),
            gen: None,
            last_sources: None,
            gen_handle: None,
            model_load: None,
            downloads: HashMap::new(),
            cpu_cores,
            ram_warning,
            gpu_info,
            api_server: None,
            vector_store,
            sandbox,
            show_kb_panel: false,
            kb_title: String::new(),
            kb_content: String::new(),
            kb_url: String::new(),
            kb_crawl_depth: 1,
            kb_indexing: false,
            kb_index_progress: 0.0,
            kb_index_status: String::new(),
            kb_job: None,
            kb_search_query: String::new(),
            kb_search_results: Vec::new(),
            kb_search_done: false,
            show_search_panel: false,
            search_query: String::new(),
            search_results: Vec::new(),
            search_loading: false,
            search_error: None,
            search_rx: None,
            conv_filter: String::new(),
            visible_msg_count: DEFAULT_VISIBLE_MSGS,
            show_model_select: false,
            show_settings: false,
            status_message: "就绪".into(),
            error_message: None,
            theme_applied: false,
            confirm_action: None,
            // Conversation list cache: avoids a SQLite query every frame in
            // the sidebar. Marked dirty by every conversation-mutating action.
            conv_cache: Vec::new(),
            conv_cache_dirty: true,
            // Current model display name (None = nothing loaded yet).
            loaded_model_name: None,
        };

        // Auto-load the previously selected model so a restart (or a
        // portable/USB run) drops straight into a ready chat instead of
        // showing "请先加载模型". Loading runs on a background thread.
        if app.config.selected_model_id.is_some() {
            app.load_selected_model();
        }
        app
    }

    pub(crate) fn is_generating(&self) -> bool {
        self.gen.is_some()
    }
}
