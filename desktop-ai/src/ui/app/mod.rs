mod chat;
mod kb_panel;
mod model_select;
mod settings;
mod sidebar;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use crate::api_server::ApiServer;
use crate::config::{self, Config};
use crate::conversation::Conversation;
use crate::crawler::extract_pdf_safe;
use crate::downloader::{self, DownloadMsg};
use crate::inference::{self, LlamaInference, StreamToken};
use crate::model_catalog::find_model;
use crate::sandbox::Sandbox;
use crate::vector_store::VectorStore;
use eframe::egui;
use egui::{Color32, RichText};

pub(crate) fn apply_theme(ctx: &egui::Context, theme: &str) {
    let mut visuals = if theme == "dark" {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = None;
    ctx.set_visuals(visuals);
}

// ─── Download state ────────────────────────────────────

pub(crate) struct DownloadState {
    pub(crate) progress: f32,
    pub(crate) status: String,
    rx: mpsc::Receiver<DownloadMsg>,
    pub(crate) cancel: Arc<AtomicBool>,
}

// ─── Generation state ──────────────────────────────────

pub(crate) struct GenState {
    pub(crate) conv_id: String,
    pub(crate) pending_text: String,
    rx: mpsc::Receiver<StreamToken>,
    pub(crate) stop_flag: Arc<AtomicBool>,
}

// ─── KB index state (background indexing) ──────────────

/// 后台知识库索引任务状态：UI 线程轮询 `rx`，`cancel` 可随时中止。
pub(crate) struct KbJobState {
    rx: mpsc::Receiver<KbIndexMsg>,
    pub(crate) cancel: Arc<AtomicBool>,
}

/// 后台索引线程 → UI 线程的消息。
enum KbIndexMsg {
    Progress {
        frac: f32,
        status: String,
    },
    Done {
        status: String,
        status_message: String,
        clear_url: bool,
        clear_title: bool,
        clear_content: bool,
    },
    Error(String),
}

/// 待执行的索引任务：UI 线程只做参数准备与校验，重活在后台线程完成。
enum KbIndexJob {
    File {
        path: PathBuf,
        filename: String,
        ext: String,
    },
    Paste {
        title: String,
        content: String,
    },
    Crawl {
        url: String,
        depth: u32,
    },
}

// ─── Model load state (off-UI-thread loading) ──────────

enum ModelLoadResult {
    Loaded {
        inference: LlamaInference,
        embedding: Option<crate::embedding::EmbeddingEngine>,
        model_name: String,
        gpu_tag: String,
    },
    Error(String),
}

pub(crate) struct ModelLoadState {
    rx: mpsc::Receiver<ModelLoadResult>,
}

// ─── App ───────────────────────────────────────────────

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
    let lib_name = crate::ffi::llama_library_name();
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
    if crate::config::should_show_crash_notice() {
        return Some(StartupNotice::CrashRecovery);
    }

    // 3. Selected model file missing.
    if let Some(id) = &config.selected_model_id {
        if let Some(info) = crate::model_catalog::find_model(&config.model_catalog, id) {
            if !crate::config::models_dir().join(&info.filename).exists() {
                return Some(StartupNotice::ModelMissing(info.name.clone()));
            }
        }
    }

    // 4. GPU configured but the library has no BLAS backend.
    if config.gpu_layers > 0 && !crate::ffi::gpu_backend_available() {
        return Some(StartupNotice::GpuUnavailable);
    }

    None
}

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
    pub(crate) kb_search_results: Vec<crate::vector_store::SearchHit>,
    pub(crate) kb_search_done: bool,

    // Search
    pub(crate) show_search_panel: bool,
    pub(crate) search_query: String,
    pub(crate) search_results: Vec<crate::search::SearchResult>,
    pub(crate) search_loading: bool,
    pub(crate) search_error: Option<String>,
    pub(crate) search_rx: Option<mpsc::Receiver<Result<Vec<crate::search::SearchResult>, String>>>,
    pub(crate) conv_filter: String,

    // UI
    pub(crate) show_model_select: bool,
    pub(crate) show_settings: bool,
    pub(crate) status_message: String,
    pub(crate) error_message: Option<String>,
    pub(crate) theme_applied: bool,
    pub(crate) confirm_action: Option<ConfirmAction>,
    /// Conversation list cache (sidebar). Refreshed only when dirty.
    pub(crate) conv_cache: Vec<crate::conversation::ConversationMeta>,
    pub(crate) conv_cache_dirty: bool,
    /// Display name of the currently loaded model (None = not loaded).
    pub(crate) loaded_model_name: Option<String>,
}

#[derive(Clone, PartialEq)]
pub(crate) enum ConfirmAction {
    DeleteAllModels,
    DeleteAllConversations,
    ResetApp,
    UninstallApp,
}

/// 成功收尾信息：文案由后台线程生成，UI 线程只负责应用。
struct KbDoneInfo {
    single_status: String,
    single_message: String,
    clear_url: bool,
    clear_title: bool,
    clear_content: bool,
}

/// 后台知识库索引线程入口：按任务类型执行读取/爬取、分块、向量化与入库，
/// 通过 `tx` 上报进度与结果；UI 线程由 [`DesktopAI::poll_kb_job`] 接收。
fn run_kb_job(
    store: Arc<VectorStore>,
    job: KbIndexJob,
    cancel: Arc<AtomicBool>,
    tx: mpsc::Sender<KbIndexMsg>,
) {
    match job {
        KbIndexJob::File {
            path,
            filename,
            ext,
        } => {
            let content = match ext.as_str() {
                "pdf" => match extract_pdf_safe(&path) {
                    Ok(t) => t,
                    Err(e) => {
                        let _ = tx.send(KbIndexMsg::Error(format!("PDF解析失败: {}", e)));
                        return;
                    }
                },
                _ => match std::fs::read_to_string(&path) {
                    Ok(t) => t,
                    Err(e) => {
                        let _ = tx.send(KbIndexMsg::Error(format!("读取失败: {}", e)));
                        return;
                    }
                },
            };
            let _ = tx.send(KbIndexMsg::Progress {
                frac: 0.2,
                status: "分块中...".into(),
            });
            let result = index_content(&store, &filename, &content, 500, 50, &cancel, &tx);
            finish_kb_job(
                &tx,
                result,
                KbDoneInfo {
                    single_status: format!("已添加: {}", filename),
                    single_message: format!("已索引文档: {}", filename),
                    clear_url: false,
                    clear_title: false,
                    clear_content: false,
                },
            );
        }
        KbIndexJob::Paste { title, content } => {
            let char_count = content.chars().count();
            let _ = tx.send(KbIndexMsg::Progress {
                frac: 0.2,
                status: format!("分块中... ({:.0} 字符)", char_count as f64),
            });
            let result = index_content(&store, &title, &content, 512, 64, &cancel, &tx);
            finish_kb_job(
                &tx,
                result,
                KbDoneInfo {
                    single_status: "完成".into(),
                    single_message: format!("已添加文档: {}", title),
                    clear_url: false,
                    clear_title: true,
                    clear_content: true,
                },
            );
        }
        KbIndexJob::Crawl { url, depth } => {
            let _ = tx.send(KbIndexMsg::Progress {
                frac: 0.05,
                status: if depth > 1 {
                    format!("深度爬取(≤{}层): {}", depth, url)
                } else {
                    format!("正在爬取: {}", url)
                },
            });
            let results = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if depth > 1 {
                    let config = crate::crawler::CrawlConfig {
                        max_depth: depth,
                        max_pages: 15,
                        ..Default::default()
                    };
                    crate::crawler::crawl_with_depth(&url, config)
                } else {
                    vec![crate::crawler::crawl_url(&url)]
                }
            }))
            .unwrap_or_else(|_| {
                // A panic inside the crawler must not kill the job silently.
                let _ = tx.send(KbIndexMsg::Error("爬取过程发生内部错误，已中止".into()));
                Vec::new()
            });

            let total = results.len();
            let mut added = 0usize;
            for result in &results {
                if cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(KbIndexMsg::Error("已取消".into()));
                    return;
                }
                match result {
                    Ok(page) => {
                        let _ = tx.send(KbIndexMsg::Progress {
                            frac: 0.3 + (added as f32 / total as f32) * 0.6,
                            status: format!(
                                "索引 {}/{}: {}",
                                added + 1,
                                total,
                                &page.title[..page.title.len().min(30)]
                            ),
                        });
                        if let Err(e) = store.add_document(&page.title, &page.text, 500, 50) {
                            log::warn!("索引失败 {}: {}", page.title, e);
                        }
                        added += 1;
                    }
                    Err(e) => {
                        log::warn!("爬取失败: {}", e);
                    }
                }
            }

            if added > 0 {
                let _ = tx.send(KbIndexMsg::Done {
                    status: format!("完成: {} 个文档已索引", added),
                    status_message: format!("已爬取 {} 个文档", added),
                    clear_url: true,
                    clear_title: false,
                    clear_content: false,
                });
            } else {
                let _ = tx.send(KbIndexMsg::Error(
                    "未爬取到有效内容。页面可能需 JavaScript 渲染，或 URL 不正确。".into(),
                ));
            }
        }
    }
}

/// 分块 + 向量化 + 入库，返回 `(成功段数, 总段数)`。
/// 大文档自动分段并逐段上报进度；单段文档直接入库。
fn index_content(
    store: &VectorStore,
    title: &str,
    content: &str,
    chunk_size: usize,
    overlap: usize,
    cancel: &AtomicBool,
    tx: &mpsc::Sender<KbIndexMsg>,
) -> Result<(usize, usize), String> {
    let char_count = content.chars().count();
    if char_count > config::KB_SINGLE_DOC_CHARS {
        let chunks = crate::chunker::chunk_text(content, chunk_size, overlap);
        let total = chunks.len();
        let mut added = 0usize;
        for (i, chunk) in chunks.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err("已取消".into());
            }
            let _ = tx.send(KbIndexMsg::Progress {
                frac: 0.3 + (i as f32 / total as f32) * 0.65,
                status: format!("索引分段 {}/{}", i + 1, total),
            });
            let seg_title = format!("{} 段{}", title, i + 1);
            match store.add_document(&seg_title, chunk, chunk_size, overlap) {
                Ok(()) => added += 1,
                Err(e) => log::warn!("索引段失败 {}: {}", seg_title, e),
            }
        }
        Ok((added, total))
    } else {
        store
            .add_document(title, content, chunk_size, overlap)
            .map_err(|e| format!("索引失败: {}", e))?;
        Ok((1, 1))
    }
}

/// 统一发送索引收尾消息：成功 → Done，失败 → Error。
fn finish_kb_job(
    tx: &mpsc::Sender<KbIndexMsg>,
    result: Result<(usize, usize), String>,
    info: KbDoneInfo,
) {
    match result {
        Ok((added, total)) => {
            if total > 1 {
                let _ = tx.send(KbIndexMsg::Done {
                    status: "完成".into(),
                    status_message: format!("文档较长，已自动切分为 {}/{} 段索引", added, total),
                    clear_url: info.clear_url,
                    clear_title: info.clear_title,
                    clear_content: info.clear_content,
                });
            } else {
                let _ = tx.send(KbIndexMsg::Done {
                    status: info.single_status,
                    status_message: info.single_message,
                    clear_url: info.clear_url,
                    clear_title: info.clear_title,
                    clear_content: info.clear_content,
                });
            }
        }
        Err(e) => {
            let _ = tx.send(KbIndexMsg::Error(e));
        }
    }
}

fn detect_hardware() -> (usize, Option<String>) {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);

    let ram_gb = get_total_ram_gb();
    let warning = if ram_gb > 0.0 && ram_gb < 4.0 {
        Some(format!(
            "⚠ 检测到内存仅 {:.1} GB，建议只使用 0.5B 或 1.7B 模型。大模型会严重卡顿或无法加载。",
            ram_gb
        ))
    } else if ram_gb > 0.0 && ram_gb < 8.0 {
        Some(format!(
            "ℹ 检测到内存 {:.1} GB，可使用 3B 以下的模型。7B+ 模型需要 8GB 以上内存。",
            ram_gb
        ))
    } else {
        None
    };
    (cores, warning)
}

#[cfg(windows)]
fn get_total_ram_gb() -> f64 {
    use std::mem;
    unsafe {
        let mut mem_status: windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX =
            mem::zeroed();
        mem_status.dwLength =
            mem::size_of::<windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX>() as u32;
        if windows_sys::Win32::System::SystemInformation::GlobalMemoryStatusEx(&mut mem_status) != 0
        {
            mem_status.ullTotalPhys as f64 / (1024.0 * 1024.0 * 1024.0)
        } else {
            0.0
        }
    }
}

#[cfg(not(windows))]
fn get_total_ram_gb() -> f64 {
    0.0
}

#[derive(Clone, Debug)]
pub(crate) struct GpuInfo {
    pub name: String,
    pub vram_gb: f64,
}

#[cfg(windows)]
fn detect_gpus() -> Vec<GpuInfo> {
    let output = std::process::Command::new("wmic")
        .args([
            "path",
            "Win32_VideoController",
            "get",
            "Name,AdapterRAM",
            "/format:csv",
        ])
        .output();
    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut gpus = Vec::new();
            for line in text.lines().skip(2) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() >= 3 {
                    let name = parts[1].trim().to_string();
                    let ram_bytes: u64 = parts[2].trim().parse().unwrap_or(0);
                    let vram = ram_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                    if vram > 0.0 && !name.is_empty() && !name.contains("Microsoft Basic") {
                        gpus.push(GpuInfo {
                            name,
                            vram_gb: vram,
                        });
                    }
                }
            }
            gpus
        }
        Err(_) => Vec::new(),
    }
}

#[cfg(not(windows))]
fn detect_gpus() -> Vec<GpuInfo> {
    Vec::new()
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
        let show_shortcut_prompt = !config.shortcut_prompted && !crate::shortcut::shortcut_exists();

        let mut app = Self {
            config,
            inference: None,
            current_conv,
            startup_notice,
            show_shortcut_prompt,
            input_text: String::new(),
            gen: None,
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

    pub(crate) fn load_selected_model(&mut self) {
        if self.model_load.is_some() {
            return;
        }
        let model_id = match &self.config.selected_model_id {
            Some(id) => id.clone(),
            None => return,
        };
        let info = match find_model(&self.config.model_catalog, &model_id) {
            Some(i) => i.clone(),
            None => return,
        };
        let model_path = config::models_dir().join(&info.filename);
        if !model_path.exists() {
            return;
        }

        self.status_message = format!("加载 {}...", info.name);
        let n_ctx = self.config.n_ctx;
        let n_threads = if self.config.n_threads == "auto" {
            std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(4)
        } else {
            self.config.n_threads.parse().unwrap_or(4)
        };
        let gpu_layers = self.config.gpu_layers;
        let kb_enabled = self.config.kb_enabled;
        let path_str = model_path.to_string_lossy().to_string();
        let model_name = info.name.clone();

        let (tx, rx) = mpsc::channel();
        self.model_load = Some(ModelLoadState { rx });

        thread::spawn(move || {
            match LlamaInference::load_ex(&path_str, n_ctx, n_threads, gpu_layers) {
                Ok(inf) => {
                    let gpu_tag = if gpu_layers > 0 {
                        format!(" [GPU {}层]", gpu_layers)
                    } else {
                        String::new()
                    };
                    let embedding = if kb_enabled {
                        match crate::embedding::EmbeddingEngine::load(&path_str, 2048, n_threads) {
                            Ok(e) => Some(e),
                            Err(e) => {
                                log::warn!("embedding engine failed: {}", e);
                                None
                            }
                        }
                    } else {
                        None
                    };
                    let _ = tx.send(ModelLoadResult::Loaded {
                        inference: inf,
                        embedding,
                        model_name,
                        gpu_tag,
                    });
                }
                Err(e) => {
                    let _ = tx.send(ModelLoadResult::Error(e));
                }
            }
        });
    }

    pub(crate) fn poll_model_load(&mut self) {
        let load = match self.model_load.as_ref() {
            Some(l) => l,
            None => return,
        };
        let result = match load.rx.try_recv() {
            Ok(r) => r,
            Err(_) => return,
        };
        self.model_load = None;
        match result {
            ModelLoadResult::Loaded {
                inference,
                embedding,
                model_name,
                gpu_tag,
            } => {
                let inf: Arc<Mutex<LlamaInference>> = Arc::new(Mutex::new(inference));
                if self.config.api_enabled {
                    if let Some(ref mut old) = self.api_server {
                        old.stop();
                    }
                    let port = self.config.api_port;
                    let token = self.config.api_token.clone();
                    let server =
                        ApiServer::start(Arc::clone(&inf), port, model_name.clone(), token);
                    self.api_server = Some(server);
                    self.status_message = format!(
                        "{} 就绪{} | API: http://127.0.0.1:{}/v1",
                        model_name, gpu_tag, port
                    );
                } else {
                    self.status_message = format!("{} 就绪{}", model_name, gpu_tag);
                }
                if let Some(engine) = embedding {
                    // 此时没有任何后台任务共享 vector_store（KB 索引需要
                    // engine 才能启动），get_mut 必然成功。
                    if let Some(store) = Arc::get_mut(&mut self.vector_store) {
                        store.set_engine(engine);
                    }
                }
                self.inference = Some(inf);
                self.loaded_model_name = Some(model_name.clone());
                log::info!("model loaded: {}", model_name);
            }
            ModelLoadResult::Error(e) => {
                log::error!("model load failed: {}", e);
                self.status_message = format!("加载失败: {}", e);
                // Surface the failure prominently instead of only in the
                // status line, so "模型未加载" is never a mystery.
                self.error_message = Some(format!("模型加载失败: {}", e));
            }
        }
    }

    pub(crate) fn delete_kb_document(&mut self, id: &str) {
        if let Err(e) = self.vector_store.delete_document(id) {
            self.error_message = Some(format!("删除失败: {}", e));
        }
    }

    pub(crate) fn pick_and_index_file(&mut self) {
        if self.kb_indexing {
            return;
        }
        if !self.vector_store.has_engine() {
            self.error_message = Some("需要先加载模型才能使用知识库".into());
            return;
        }
        #[cfg(not(target_os = "android"))]
        let path = if let Some(p) = rfd::FileDialog::new()
            .add_filter("文档", &["txt", "md", "pdf"])
            .pick_file()
        {
            p
        } else {
            let filepath = self.kb_title.trim().to_string();
            if filepath.is_empty() {
                return;
            }
            let p = std::path::PathBuf::from(&filepath);
            if !p.exists() {
                self.error_message = Some(format!("文件不存在: {}", filepath));
                return;
            }
            p
        };
        #[cfg(target_os = "android")]
        let path = {
            // Android: no native file dialog — use the typed path instead.
            let filepath = self.kb_title.trim().to_string();
            if filepath.is_empty() {
                self.error_message = Some("请输入文件路径".into());
                return;
            }
            let p = std::path::PathBuf::from(&filepath);
            if !p.exists() {
                self.error_message = Some(format!("文件不存在: {}", filepath));
                return;
            }
            p
        };

        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        // ── File type whitelist ──
        if !matches!(ext.as_str(), "txt" | "md" | "pdf") {
            self.error_message = Some(format!("不支持的文件类型 .{} — 仅支持 .txt .md .pdf", ext));
            return;
        }

        // ── Binary sniff (first 10 bytes) ──
        if ext != "pdf" {
            if let Ok(mut f) = std::fs::File::open(&path) {
                use std::io::Read;
                let mut head = [0u8; 10];
                let n = f.read(&mut head).unwrap_or(0);
                let zeros = head[..n].iter().filter(|&&b| b == 0).count();
                if n >= 4 && zeros * 2 >= n {
                    self.error_message = Some("请转换为纯文本格式".into());
                    return;
                }
            }
        }

        // 读取/分块/向量化/入库全部在后台线程执行，避免 UI 卡顿。
        self.start_kb_job(KbIndexJob::File {
            path,
            filename,
            ext,
        });
    }

    pub(crate) fn paste_and_index_text(&mut self) {
        let title = self.kb_title.trim().to_string();
        let content = self.kb_content.trim().to_string();
        if title.is_empty() || content.is_empty() {
            return;
        }
        if !self.vector_store.has_engine() {
            self.error_message = Some("需要先加载模型才能使用知识库".into());
            return;
        }
        // 读取/分块/向量化/入库全部在后台线程执行，避免 UI 卡顿。
        self.start_kb_job(KbIndexJob::Paste { title, content });
    }

    pub(crate) fn crawl_url_to_kb(&mut self) {
        if self.kb_indexing {
            return;
        }
        let url = self.kb_url.trim().to_string();
        if url.is_empty() {
            return;
        }
        if !self.vector_store.has_engine() {
            self.error_message = Some("需要先加载模型才能使用知识库".into());
            return;
        }
        let depth = self.kb_crawl_depth.clamp(1, 3);
        // 爬取/分块/向量化/入库全部在后台线程执行，避免 UI 卡顿。
        self.start_kb_job(KbIndexJob::Crawl { url, depth });
    }

    /// 启动后台知识库索引任务（三种入口共用）。
    fn start_kb_job(&mut self, job: KbIndexJob) {
        let store = Arc::clone(&self.vector_store);
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        self.kb_job = Some(KbJobState {
            rx,
            cancel: cancel.clone(),
        });
        self.kb_indexing = true;
        self.kb_index_progress = 0.0;
        self.kb_index_status = match &job {
            KbIndexJob::File { .. } => "读取文件...".into(),
            KbIndexJob::Paste { .. } => "正在向量化...".into(),
            KbIndexJob::Crawl { depth, url } if *depth > 1 => {
                format!("深度爬取(≤{}层): {}", depth, url)
            }
            KbIndexJob::Crawl { url, .. } => format!("正在爬取: {}", url),
        };
        thread::spawn(move || run_kb_job(store, job, cancel, tx));
    }

    /// 轮询后台索引任务：更新进度/状态，任务结束（Done/Error）时恢复 UI。
    pub(crate) fn poll_kb_job(&mut self) {
        let job = match self.kb_job.take() {
            Some(j) => j,
            None => return,
        };
        loop {
            match job.rx.try_recv() {
                Ok(KbIndexMsg::Progress { frac, status }) => {
                    self.kb_index_progress = frac;
                    self.kb_index_status = status;
                }
                Ok(KbIndexMsg::Done {
                    status,
                    status_message,
                    clear_url,
                    clear_title,
                    clear_content,
                }) => {
                    self.kb_index_progress = 1.0;
                    self.kb_index_status = status;
                    self.status_message = status_message;
                    if clear_url {
                        self.kb_url.clear();
                    }
                    if clear_title {
                        self.kb_title.clear();
                    }
                    if clear_content {
                        self.kb_content.clear();
                    }
                    self.kb_indexing = false;
                    self.kb_job = None;
                    return;
                }
                Ok(KbIndexMsg::Error(e)) => {
                    self.error_message = Some(e);
                    self.kb_indexing = false;
                    self.kb_job = None;
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.kb_job = Some(job);
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // 线程异常退出（如 panic）：恢复 UI 状态
                    self.kb_indexing = false;
                    self.kb_job = None;
                    return;
                }
            }
        }
    }

    // ─── Concurrent downloads ──────────────────────────

    pub(crate) fn start_download(&mut self, model_id: &str) {
        if self.downloads.contains_key(model_id) {
            return;
        }

        let info = match find_model(&self.config.model_catalog, model_id) {
            Some(i) => i.clone(),
            None => return,
        };

        let dest = config::models_dir().join(&info.filename);
        let url = info.url.clone();
        let expected_sha256 = info.expected_sha256.clone();
        let parts = info.parts.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();

        self.downloads.insert(
            model_id.to_string(),
            DownloadState {
                progress: 0.0,
                status: "连接中...".into(),
                rx,
                cancel: cancel.clone(),
            },
        );

        thread::spawn(move || {
            downloader::download_model(&url, dest, cancel, tx, expected_sha256.as_deref(), &parts);
        });
    }

    pub(crate) fn cancel_download(&mut self, model_id: &str) {
        if let Some(ds) = self.downloads.get(model_id) {
            ds.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub(crate) fn poll_all_downloads(&mut self) {
        let mut finished = vec![];
        let mut errors = vec![];

        for (id, ds) in self.downloads.iter_mut() {
            while let Ok(msg) = ds.rx.try_recv() {
                match msg {
                    DownloadMsg::Progress {
                        percent,
                        downloaded_mb,
                        total_mb,
                    } => {
                        ds.progress = percent as f32 / 100.0;
                        ds.status = format!(
                            "{:.0}% ({:.0}/{:.0}MB)",
                            percent as f64, downloaded_mb, total_mb
                        );
                    }
                    DownloadMsg::Status(s) => {
                        ds.status = s;
                    }
                    DownloadMsg::Done => {
                        finished.push(id.clone());
                        break;
                    }
                    DownloadMsg::Error(e) => {
                        errors.push((id.clone(), e));
                        break;
                    }
                }
            }
        }

        for id in finished {
            self.downloads.remove(&id);
            // Auto-load: either the downloaded model was already selected,
            // or nothing is loaded yet — pick the fresh model so a user who
            // downloaded models from the USB/portable build can chat right
            // away without hunting through the model picker.
            let already_selected = self.config.selected_model_id.as_deref() == Some(&id);
            let nothing_loaded = self.inference.is_none() && self.model_load.is_none();
            if already_selected || nothing_loaded {
                if !already_selected {
                    self.config.selected_model_id = Some(id.clone());
                    config::save_config(&self.config);
                }
                self.load_selected_model();
            }
        }
        for (id, err) in errors {
            self.downloads.remove(&id);
            self.error_message = Some(format!("下载 {} 失败: {}", id, err));
        }
    }

    // ─── Generation ────────────────────────────────────

    pub(crate) fn send_message(&mut self) {
        if self.is_generating() {
            return;
        }
        let text = self.input_text.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.input_text.clear();

        self.current_conv.add_message("user", &text);

        let inf = match self.inference.as_ref() {
            Some(inf) => Arc::clone(inf),
            None => {
                self.status_message = "模型未加载".into();
                return;
            }
        };

        let messages = self
            .current_conv
            .context_messages(Some(&self.config.system_prompt), 20);
        let conv_id = self.current_conv.id.clone();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let do_search = self.config.search_enabled;
        // 索引进行中跳过 KB 注入：embedding 锁被后台任务持有，
        // 避免 UI 线程等待整个文档的向量化过程。
        let do_kb =
            self.config.kb_enabled && self.vector_store.has_engine() && self.kb_job.is_none();
        let user_query = text.clone();

        let kb_data = if do_kb {
            self.vector_store.documents_snapshot()
        } else {
            Vec::new()
        };
        let query_vec = if do_kb {
            self.vector_store.embed_query(&user_query).ok()
        } else {
            None
        };

        let stop = stop_flag.clone();

        self.gen = Some(GenState {
            conv_id,
            pending_text: String::new(),
            rx,
            stop_flag,
        });

        let handle = thread::spawn(move || {
            let kb_context = if let Some(ref qv) = query_vec {
                if !kb_data.is_empty() {
                    let results = crate::vector_store::search_by_vector(&kb_data, qv, 3);
                    if !results.is_empty() {
                        let mut ctx = String::new();
                        for (i, hit) in results.iter().enumerate() {
                            ctx.push_str(&format!(
                                "[参考{} 来源: {} 相似度{:.0}%]\n{}\n\n",
                                i + 1,
                                hit.source,
                                hit.score * 100.0,
                                hit.chunk
                            ));
                        }
                        Some(ctx)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            let search_context = if do_search {
                if let Ok(results) = crate::search::search_duckduckgo(&user_query) {
                    if !results.is_empty() {
                        let mut ctx = String::new();
                        for (i, r) in results.iter().take(5).enumerate() {
                            ctx.push_str(&format!("[结果{}] {}\n", i + 1, r.title));
                            if !r.snippet.is_empty() {
                                ctx.push_str(&format!("  摘要: {}\n", r.snippet));
                            }
                            if !r.url.is_empty() {
                                ctx.push_str(&format!("  来源: {}\n", r.url));
                            }
                            ctx.push('\n');
                        }
                        Some(ctx)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            let prompt = inference::build_rag_prompt(
                &messages,
                kb_context.as_deref(),
                search_context.as_deref(),
            );

            inference::run_inference(inf, prompt, stop, tx, 2048);
        });
        self.gen_handle = Some(handle);
    }

    pub(crate) fn stop_generation(&mut self) {
        if let Some(ref gen) = self.gen {
            gen.stop_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub(crate) fn poll_generation(&mut self) {
        let gen = match self.gen.as_mut() {
            Some(g) => g,
            None => return,
        };

        let mut done = false;
        while let Ok(token) = gen.rx.try_recv() {
            match token {
                StreamToken::Text(t) => {
                    gen.pending_text.push_str(&t);
                    // Repetition detector: if the last 50 characters are
                    // identical the model is stuck in a token loop.
                    let window: Vec<char> = gen.pending_text.chars().rev().take(50).collect();
                    if window.len() == 50 && window.iter().all(|&c| c == window[0]) {
                        gen.stop_flag
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        gen.pending_text
                            .push_str("\n\n[检测到重复输出，已自动停止]");
                        done = true;
                        break;
                    }
                }
                StreamToken::Done => {
                    done = true;
                    break;
                }
                StreamToken::Error(e) => {
                    gen.pending_text.push_str(&format!("\n\n*[错误: {}]*", e));
                    done = true;
                    break;
                }
            }
        }

        if done {
            let response = gen.pending_text.clone();
            let conv_id = gen.conv_id.clone();

            if let Some(mut conv) = Conversation::load(&conv_id) {
                conv.add_message("assistant", &response);
                self.conv_cache_dirty = true;
            }

            self.gen = None;
            if let Some(handle) = self.gen_handle.take() {
                let _ = handle.join();
            }
        }
    }

    // ─── Conversation management ────────────────────────

    /// Refresh the cached conversation list (lazy: only when dirty).
    pub(crate) fn refresh_conv_cache(&mut self) {
        if self.conv_cache_dirty {
            self.conv_cache = Conversation::list_all();
            self.conv_cache_dirty = false;
        }
    }

    pub(crate) fn new_conversation(&mut self) {
        self.current_conv = Conversation::new();
        self.conv_cache_dirty = true;
    }

    pub(crate) fn load_conversation(&mut self, id: &str) {
        if let Some(conv) = Conversation::load(id) {
            self.current_conv = conv;
            self.config.last_conversation_id = Some(id.to_string());
            config::save_config(&self.config);
        }
    }

    pub(crate) fn delete_conversation(&mut self, id: &str) {
        Conversation::delete(id);
        self.conv_cache_dirty = true;
        if self.current_conv.id == id {
            self.current_conv = Conversation::new();
        }
    }

    pub(crate) fn export_current_conversation(&mut self) {
        if self.current_conv.messages.is_empty() {
            self.error_message = Some("当前对话为空，无法导出".into());
            return;
        }
        let default_name = format!("conversation_{}.json", self.current_conv.id);
        #[cfg(not(target_os = "android"))]
        let path = match rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("JSON", &["json"])
            .save_file()
        {
            Some(p) => p,
            None => return,
        };
        #[cfg(target_os = "android")]
        let path = {
            // Android: no native save dialog — write to the app data dir.
            crate::config::data_root().join(&default_name)
        };
        match self.current_conv.export_json() {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    self.error_message = Some(format!("写入失败: {}", e));
                    return;
                }
                self.error_message = Some(format!("已导出至 {:?}", path));
            }
            Err(e) => self.error_message = Some(e),
        }
    }

    #[cfg(not(target_os = "android"))]
    pub(crate) fn import_conversation(&mut self) {
        let path = match rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .pick_file()
        {
            Some(p) => p,
            None => return,
        };
        let content = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                self.error_message = Some(format!("读取失败: {}", e));
                return;
            }
        };
        match Conversation::import_json(&content) {
            Ok(mut conv) => {
                conv.save();
                self.current_conv = conv;
                self.conv_cache_dirty = true;
                self.error_message = Some("导入成功".into());
            }
            Err(e) => self.error_message = Some(e),
        }
    }

    /// Android has no native file picker.
    #[cfg(target_os = "android")]
    pub(crate) fn import_conversation(&mut self) {
        self.error_message = Some("安卓端暂不支持导入对话".into());
    }

    pub(crate) fn delete_model_file(&mut self, model_id: &str) {
        if let Some(info) = find_model(&self.config.model_catalog, model_id) {
            let path = config::models_dir().join(&info.filename);
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("failed to delete model file {:?}: {}", path, e);
            }
        }
    }

    pub(crate) fn delete_all_models(&mut self) {
        let dir = config::models_dir();
        if dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                log::warn!("failed to delete models dir: {}", e);
            }
        }
        self.inference = None;
        self.config.selected_model_id = None;
        config::save_config(&self.config);
        self.status_message = "所有模型已删除".into();
    }

    pub(crate) fn delete_all_conversations(&mut self) {
        Conversation::delete_all();
        self.current_conv = Conversation::new();
        self.gen = None;
        self.status_message = "所有对话已删除".into();
    }

    pub(crate) fn reset_app(&mut self) {
        let app_dir = config::data_root();
        if let Err(e) = std::fs::remove_dir_all(&app_dir) {
            log::warn!("reset_app: failed to remove data dir: {}", e);
        }
        let config_file = config::config_path();
        if let Err(e) = std::fs::remove_file(&config_file) {
            log::warn!("reset_app: failed to remove config file: {}", e);
        }
        self.inference = None;
        self.config = Config::default();
        self.current_conv = Conversation::new();
        self.gen = None;
        self.downloads.clear();
        self.show_settings = false;
        self.status_message = "应用已重置。请重新选择模型。".into();
    }

    pub(crate) fn uninstall_app(&mut self) {
        let app_dir = config::data_root();
        let _ = std::fs::remove_dir_all(&app_dir);
        let config_file = config::config_path();
        let _ = std::fs::remove_file(&config_file);

        let exe_path = std::env::current_exe().unwrap_or_default();
        let exe_dir = exe_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let lib_path = exe_dir.join(crate::ffi::llama_library_name());

        #[cfg(windows)]
        {
            // Windows cannot delete a running executable, so hand the
            // deletion off to a background batch script.
            let batch_path = exe_dir.join("_uninstall.bat");
            if let Ok(mut f) = std::fs::File::create(&batch_path) {
                use std::io::Write;
                let _ = writeln!(f, "@echo off");
                let _ = writeln!(f, "echo 桌面AI 卸载中...");
                let _ = writeln!(f, "timeout /t 2 /nobreak >nul");
                let _ = writeln!(f, "del /f /q \"{}\"", exe_path.display());
                let _ = writeln!(f, "del /f /q \"{}\"", lib_path.display());
                let _ = writeln!(f, "del /f /q \"{}\"", batch_path.display());
                let _ = writeln!(f, "echo 桌面AI 已卸载。");
                let _ = writeln!(f, "timeout /t 2 /nobreak >nul");
            }

            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "/MIN", ""])
                .arg(batch_path.to_string_lossy().to_string())
                .spawn();
        }

        #[cfg(not(windows))]
        {
            // Unix allows unlinking an executable that is currently running.
            let _ = std::fs::remove_file(&exe_path);
            let _ = std::fs::remove_file(&lib_path);
            for name in ["libllama.so.0", "libggml.so.0", "libggml-base.so.0"] {
                let _ = std::fs::remove_file(exe_dir.join(name));
            }
        }

        std::process::exit(0);
    }

    pub(crate) fn start_search(&mut self) {
        let query = self.search_query.trim().to_string();
        if query.is_empty() || self.search_loading {
            return;
        }
        self.search_loading = true;
        self.search_error = None;
        self.search_results.clear();
        let (tx, rx) = mpsc::channel();
        self.search_rx = Some(rx);
        thread::spawn(move || {
            let _ = tx.send(crate::search::search_duckduckgo(&query));
        });
    }

    pub(crate) fn poll_search(&mut self) {
        let rx = match self.search_rx.as_mut() {
            Some(r) => r,
            None => return,
        };
        if let Ok(result) = rx.try_recv() {
            match result {
                Ok(r) => self.search_results = r,
                Err(e) => self.search_error = Some(e),
            }
            self.search_loading = false;
            self.search_rx = None;
        }
    }
}

// ─── egui App ──────────────────────────────────────────

impl eframe::App for DesktopAI {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_model_load();
        self.poll_all_downloads();
        self.poll_generation();
        self.poll_search();
        self.poll_kb_job();

        // Keyboard shortcuts
        let input = ctx.input(|i| i.clone());
        if input.modifiers.ctrl {
            if input.key_pressed(egui::Key::N) {
                self.new_conversation();
            }
            if input.key_pressed(egui::Key::F) {
                self.show_search_panel = true;
            }
        }
        if input.key_pressed(egui::Key::Escape) {
            if self.show_settings {
                self.show_settings = false;
            } else if self.show_model_select {
                self.show_model_select = false;
            } else if self.show_search_panel {
                self.show_search_panel = false;
            }
        }

        // Apply saved theme only once on startup
        if !self.theme_applied {
            self.theme_applied = true;
            if self.config.theme == "light" {
                ctx.set_visuals(egui::Visuals::light());
            }
        }

        // ─── Top bar ───────────────────────────────
        egui::TopBottomPanel::top("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let download_count = self.downloads.len();
                let download_status;

                let (color, text): (Color32, &str) = if self.is_generating() {
                    (Color32::from_rgb(255, 165, 0), "生成中...")
                } else if download_count > 0 {
                    download_status = format!("下载中 ({} 个)", download_count);
                    (Color32::from_rgb(255, 165, 0), download_status.as_str())
                } else if self.inference.is_some() {
                    (Color32::from_rgb(76, 175, 80), "就绪")
                } else {
                    (Color32::from_rgb(150, 150, 150), "未加载")
                };
                ui.label(RichText::new("●").color(color).size(16.0));
                ui.label(text);

                if let Some(ref id) = self.config.selected_model_id {
                    if let Some(info) = find_model(&self.config.model_catalog, id) {
                        ui.label(format!(" | {}", info.name));
                    }
                }

                if self.config.search_enabled || self.config.kb_enabled {
                    let tag = if self.config.kb_enabled && self.config.search_enabled {
                        " | RAG+KB"
                    } else if self.config.kb_enabled {
                        " | KB"
                    } else {
                        " | RAG"
                    };
                    ui.label(
                        RichText::new(tag)
                            .size(11.0)
                            .color(Color32::from_rgb(100, 200, 255)),
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let icon = if self.config.theme == "dark" {
                        "☀"
                    } else {
                        "🌙"
                    };
                    if ui.button(icon).clicked() {
                        self.config.theme = if self.config.theme == "dark" {
                            "light".into()
                        } else {
                            "dark".into()
                        };
                        apply_theme(ctx, &self.config.theme);
                        config::save_config(&self.config);
                    }
                    if ui.button("⚙").clicked() {
                        self.show_settings = true;
                    }
                });
            });

            if !self.downloads.is_empty() {
                ui.separator();
                let mut to_cancel = vec![];
                for (id, ds) in self.downloads.iter() {
                    if let Some(info) = find_model(&self.config.model_catalog, id) {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&info.name).size(11.0));
                            ui.add(
                                egui::ProgressBar::new(ds.progress)
                                    .desired_width(150.0)
                                    .text(&ds.status),
                            );
                            if ui.button("取消").clicked() {
                                to_cancel.push(id.clone());
                            }
                        });
                    }
                }
                for id in to_cancel {
                    self.cancel_download(&id);
                }
            }
        });

        // ─── Sidebar ───────────────────────────────
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .default_width(200.0)
            .show(ctx, |ui| {
                self.render_sidebar(ui);
            });

        // ─── Input bar (BEFORE CentralPanel: egui panels added after the
        // central panel overlap it instead of reducing its space — this was
        // hiding the bottom of the chat output behind the input box) ─────
        egui::TopBottomPanel::bottom("input").show(ctx, |ui| {
            self.render_input_bar(ui);
        });

        // ─── Chat area ─────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_chat_area(ctx, ui);
        });

        // ─── Model select window ───────────────────
        if self.show_model_select {
            egui::Window::new("选择模型")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    self.render_model_select(ui);
                });
        }

        // ─── Search panel ──────────────────────────
        if self.show_search_panel {
            egui::Window::new("搜索")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::RIGHT_TOP, [0.0, 30.0])
                .default_width(350.0)
                .max_width(450.0)
                .show(ctx, |ui| {
                    self.render_search_panel(ui);
                });
        }

        // ─── Knowledge Base panel ────────────────────
        if self.show_kb_panel {
            egui::Window::new("知识库")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::RIGHT_TOP, [0.0, 30.0])
                .default_width(420.0)
                .max_width(500.0)
                .show(ctx, |ui| {
                    self.render_kb_panel(ui);
                });
        }

        // ─── Settings ─────────────────────────────
        if self.show_settings {
            egui::Window::new("设置")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    self.render_settings(ctx, ui);
                });
        }

        // ─── Confirm dialog ───────────────────────
        if let Some(ref action) = self.confirm_action.clone() {
            let (title, msg, is_danger) = match action {
                ConfirmAction::DeleteAllModels => ("删除所有模型", "确定要删除所有已下载的模型文件吗？此操作不可恢复。", false),
                ConfirmAction::DeleteAllConversations => ("删除所有对话", "确定要删除所有对话记录吗？此操作不可恢复。", false),
                ConfirmAction::ResetApp => ("⚠ 重置应用", "确定要删除所有数据（模型、对话、配置）？\n应用将恢复到初始状态，所有数据将永久丢失。", true),
                ConfirmAction::UninstallApp => ("⚠ 卸载应用", "确定要完全卸载桌面AI吗？\n\n将删除：\n• 所有已下载模型\n• 所有对话记录\n• 应用配置文件\n• 程序文件（exe + dll）\n\n此操作不可恢复！", true),
            };
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if is_danger {
                        ui.label(RichText::new(msg).color(Color32::from_rgb(255, 80, 80)));
                    } else {
                        ui.label(msg);
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if is_danger {
                            let (btn_text, action_copy) = match action {
                                ConfirmAction::ResetApp => ("确定重置", ConfirmAction::ResetApp),
                                ConfirmAction::UninstallApp => {
                                    ("确定卸载", ConfirmAction::UninstallApp)
                                }
                                _ => ("确定删除", ConfirmAction::ResetApp),
                            };
                            let confirm_btn =
                                egui::Button::new(RichText::new(btn_text).color(Color32::WHITE))
                                    .fill(Color32::from_rgb(192, 57, 43));
                            if ui.add(confirm_btn).clicked() {
                                match action_copy {
                                    ConfirmAction::ResetApp => self.reset_app(),
                                    ConfirmAction::UninstallApp => self.uninstall_app(),
                                    _ => {}
                                }
                                self.confirm_action = None;
                                self.show_settings = false;
                            }
                        } else {
                            if ui.button("确定").clicked() {
                                match action {
                                    ConfirmAction::DeleteAllModels => self.delete_all_models(),
                                    ConfirmAction::DeleteAllConversations => {
                                        self.delete_all_conversations()
                                    }
                                    _ => {}
                                }
                                self.confirm_action = None;
                                self.show_settings = false;
                            }
                        }
                        if ui.button("取消").clicked() {
                            self.confirm_action = None;
                        }
                    });
                });
        }

        // ─── Startup notice (single, highest-priority popup) ──
        if let Some(ref notice) = self.startup_notice.clone() {
            let (title, body) = match notice {
                StartupNotice::MissingDependency(lib) => (
                    "启动检查：缺少依赖",
                    format!(
                        "未找到运行时依赖库 {}。\n\n请从 GitHub Releases 重新下载完整安装包，\
                         或确认 {} 与程序位于同一目录。",
                        lib, lib
                    ),
                ),
                StartupNotice::CrashRecovery => (
                    "上次异常退出",
                    format!(
                        "检测到上次运行异常退出（崩溃或强制结束）。\n\n您的对话已实时自动保存，\
                         可以放心继续使用。\n崩溃日志位于：\n{}",
                        config::log_dir().display()
                    ),
                ),
                StartupNotice::ModelMissing(name) => (
                    "模型文件缺失",
                    format!(
                        "上次选择的模型“{}”文件不存在。\n\n请到“切换模型”中重新下载，\
                         或选择其他模型。",
                        name
                    ),
                ),
                StartupNotice::GpuUnavailable => (
                    "GPU 加速不可用",
                    "当前已配置 GPU 加速，但检测到 llama 库为 CPU 版本，\
                     加速不会生效。\n\n可在设置中关闭 GPU 层数，\
                     或更换带 CUDA/Vulkan 后端的库。"
                        .to_string(),
                ),
            };
            let red = egui::Color32::from_rgb(230, 90, 80);
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    let critical = matches!(
                        notice,
                        StartupNotice::MissingDependency(_) | StartupNotice::CrashRecovery
                    );
                    if critical {
                        ui.label(RichText::new(&body).color(red));
                    } else {
                        ui.label(&body);
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("知道了").clicked() {
                            self.startup_notice = None;
                        }
                        if matches!(notice, StartupNotice::CrashRecovery)
                            && ui.button("打开日志目录").clicked()
                        {
                            let dir = config::log_dir();
                            #[cfg(target_os = "windows")]
                            let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                            #[cfg(target_os = "linux")]
                            let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
                            self.startup_notice = None;
                        }
                    });
                });
        }

        // ─── First-run desktop shortcut prompt ──────────
        if self.show_shortcut_prompt && self.startup_notice.is_none() {
            egui::Window::new("创建桌面快捷方式")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("是否在桌面创建一个启动快捷方式？");
                    ui.label(
                        RichText::new("(可在设置中随时重新创建)")
                            .size(10.0)
                            .color(egui::Color32::GRAY),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("创建").clicked() {
                            match crate::shortcut::create_desktop_shortcut() {
                                Ok(true) => {
                                    self.status_message = "桌面快捷方式已创建".into();
                                }
                                Ok(false) => {
                                    self.status_message = "未创建快捷方式".into();
                                }
                                Err(e) => {
                                    self.error_message = Some(format!("创建快捷方式失败: {}", e));
                                }
                            }
                            self.config.shortcut_prompted = true;
                            config::save_config(&self.config);
                            self.show_shortcut_prompt = false;
                        }
                        if ui.button("跳过").clicked() {
                            self.config.shortcut_prompted = true;
                            config::save_config(&self.config);
                            self.show_shortcut_prompt = false;
                        }
                    });
                });
        }

        // ─── Error toast ───────────────────────────
        if let Some(ref err) = self.error_message.clone() {
            egui::Window::new("错误")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(err);
                    if ui.button("确定").clicked() {
                        self.error_message = None;
                    }
                });
        }

        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}
