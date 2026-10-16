// DesktopAI event handlers: model loading, generation, downloads, KB ops,
// conversation management, search, and app-level destructive actions.
//
// Every public method here is called either from the eframe update loop or
// from one of the UI sub-modules (chat, sidebar, settings, ...).

use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use super::state::{
    DownloadState, GenState, KbJobState, ModelLoadResult, ModelLoadState,
};
use crate::llm::downloader::{self, DownloadMsg};
use crate::llm::inference::{self, LlamaInference, StreamToken};
use crate::llm::model_catalog::find_model;
use crate::rag::kb_job::{run_kb_job, KbIndexJob, KbIndexMsg};
use crate::server::api_server::ApiServer;
use crate::store::config::{self, Config};
use crate::store::conversation::Conversation;

use super::state::DesktopAI;

impl DesktopAI {
    // ─── Model loading ─────────────────────────────────

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
                        match crate::llm::embedding::EmbeddingEngine::load(
                            &path_str, 2048, n_threads,
                        ) {
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
                        ApiServer::start(Some(Arc::clone(&inf)), port, model_name.clone(), token);
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

    // ─── Knowledge base ────────────────────────────────

    pub(crate) fn delete_kb_document(&mut self, id: &str) {
        if let Err(e) = self.vector_store.delete_document(id) {
            self.show_toast(format!("删除失败: {}", e));
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
                self.show_toast(format!("文件不存在: {}", filepath));
                return;
            }
            p
        };
        #[cfg(target_os = "android")]
        let path = {
            // Android: no native file dialog — use the typed path instead.
            let filepath = self.kb_title.trim().to_string();
            if filepath.is_empty() {
                self.show_toast("请输入文件路径");
                return;
            }
            let p = std::path::PathBuf::from(&filepath);
            if !p.exists() {
                self.show_toast(format!("文件不存在: {}", filepath));
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
            self.show_toast(format!("不支持的文件类型 .{} — 仅支持 .txt .md .pdf", ext));
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
                    self.show_toast("请转换为纯文本格式");
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
                    self.show_toast(format!("知识库索引失败: {}", e));
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
            self.show_toast(format!("下载 {} 失败: {}", id, err));
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
        let rag_top_k = self.config.rag_top_k;
        let max_out = self.config.max_output_tokens;

        self.gen = Some(GenState {
            conv_id,
            pending_text: String::new(),
            notice: None,
            sources: Vec::new(),
            rx,
            stop_flag,
        });

        let handle = thread::spawn(move || {
            let (kb_context, kb_sources) = if let Some(ref qv) = query_vec {
                if !kb_data.is_empty() {
                    let results =
                        crate::rag::vector_store::search_by_vector(&kb_data, qv, rag_top_k);
                    if !results.is_empty() {
                        let mut ctx = String::new();
                        for (i, hit) in results.iter().enumerate() {
                            // Rank only — a cosine "similarity %" would be
                            // misleading (not a probability, can be negative).
                            ctx.push_str(&format!(
                                "[参考{}（按相关度排序） 来源: {}]\n{}\n\n",
                                i + 1,
                                hit.source,
                                hit.chunk
                            ));
                        }
                        let sources = crate::rag::vector_store::SourceRef::from_hits(&results);
                        (Some(ctx), sources)
                    } else {
                        (None, Vec::new())
                    }
                } else {
                    (None, Vec::new())
                }
            } else {
                (None, Vec::new())
            };

            let search_context = if do_search {
                if let Ok(results) = crate::rag::search::search_duckduckgo(&user_query) {
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

            inference::run_inference(
                inf,
                inference::InferenceRequest {
                    messages,
                    kb_context,
                    search_context,
                    sources: kb_sources,
                    max_tokens: max_out,
                },
                stop,
                tx,
            );
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
                StreamToken::Notice(msg) => {
                    // Context-budget notice: keep the latest one for display.
                    gen.notice = Some(msg);
                }
                StreamToken::Sources(sources) => {
                    gen.sources = sources;
                }
            }
        }

        if done {
            let response = gen.pending_text.clone();
            let conv_id = gen.conv_id.clone();
            // Keep this reply's retrieval references for the source section.
            if !gen.sources.is_empty() {
                self.last_sources = Some((conv_id.clone(), gen.sources.clone()));
            }

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
        self.msg_heights.clear();
    }

    pub(crate) fn load_conversation(&mut self, id: &str) {
        if let Some(conv) = Conversation::load(id) {
            self.current_conv = conv;
            self.config.last_conversation_id = Some(id.to_string());
            config::save_config(&self.config);
            self.msg_heights.clear();
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
            self.show_toast("当前对话为空，无法导出");
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
            crate::store::config::data_root().join(&default_name)
        };
        match self.current_conv.export_json() {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    self.show_toast(format!("导出失败: {}", e));
                    return;
                }
                self.show_toast(format!("已导出至 {:?}", path));
            }
            Err(e) => self.show_toast(format!("导出失败: {}", e)),
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
        // 大小预检：导入的是用户可选的任意本地 JSON，先查 metadata 再读取，
        // 防止 read_to_string 对巨型文件无界分配（crawler 侧已有 5MB 先例）。
        if let Err(e) =
            crate::rag::kb_job::ensure_file_size(&path, crate::rag::kb_job::MAX_IMPORT_JSON_BYTES)
        {
            self.show_toast(e);
            return;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                self.show_toast(format!("读取失败: {}", e));
                return;
            }
        };
        match Conversation::import_json(&content) {
            Ok(mut conv) => {
                conv.save();
                self.current_conv = conv;
                self.conv_cache_dirty = true;
                self.msg_heights.clear();
                self.show_toast("导入成功");
            }
            Err(e) => self.show_toast(format!("导入失败: {}", e)),
        }
    }

    /// Android has no native file picker.
    #[cfg(target_os = "android")]
    pub(crate) fn import_conversation(&mut self) {
        self.show_toast("安卓端暂不支持导入对话");
    }

    // ─── Model / data management ────────────────────────

    pub(crate) fn delete_model_file(&mut self, model_id: &str) {
        if let Some(info) = find_model(&self.config.model_catalog, model_id) {
            let dir = config::models_dir();
            let path = dir.join(&info.filename);
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("failed to delete model file {:?}: {}", path, e);
            }
            // 顺带清理未完成下载的暂存文件（<filename>.download / .meta）
            for suffix in [".download", ".download.meta"] {
                let tmp = dir.join(format!("{}{}", info.filename, suffix));
                if tmp.exists() {
                    if let Err(e) = std::fs::remove_file(&tmp) {
                        log::warn!("failed to delete download staging {:?}: {}", tmp, e);
                    }
                }
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
        let lib_path = exe_dir.join(crate::llm::ffi::llama_library_name());

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

    // ─── Search ────────────────────────────────────────

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
            let _ = tx.send(crate::rag::search::search_duckduckgo(&query));
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

    /// Poll the background model-catalog fetch (started once in `new()`).
    /// On success, merges remote entries into the local catalog and saves.
    pub(crate) fn poll_catalog_refresh(&mut self) {
        let rx = match self.catalog_rx.as_mut() {
            Some(r) => r,
            None => return,
        };
        if let Ok(result) = rx.try_recv() {
            match result {
                Ok(remote) => {
                    let n = crate::llm::model_catalog::merge_catalog(
                        &mut self.config.model_catalog,
                        remote,
                    );
                    self.catalog_refresh_status =
                        Some(format!("已从云端更新 {} 个模型条目", n));
                    config::save_config(&self.config);
                    log::info!("model catalog refreshed: {} entries merged", n);
                }
                Err(e) => {
                    self.catalog_refresh_status =
                        Some(format!("目录更新失败（使用内置目录）: {}", e));
                    log::warn!("model catalog refresh failed: {}", e);
                }
            }
            self.catalog_rx = None;
        }
    }
}
