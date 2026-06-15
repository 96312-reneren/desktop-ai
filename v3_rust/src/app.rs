use std::collections::HashMap;
use std::sync::{Arc, atomic::AtomicBool, mpsc};
use std::thread;

use eframe::egui;
use egui::{Color32, RichText, ScrollArea, TextEdit, vec2};
use crate::api_server::ApiServer;
use crate::config::{self, Config};
use crate::conversation::Conversation;
use crate::downloader::{self, DownloadMsg};
use crate::inference::{self, LlamaInference, StreamToken};
use crate::markdown;
use crate::model_catalog::find_model;
use crate::search::{self, SearchResult};

fn apply_theme(ctx: &egui::Context, theme: &str) {
    let mut visuals = if theme == "dark" {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = None;
    ctx.set_visuals(visuals);
}

// ─── Download state ────────────────────────────────────

struct DownloadState {
    progress: f32,
    status: String,
    rx: mpsc::Receiver<DownloadMsg>,
    cancel: Arc<AtomicBool>,
}

// ─── Generation state ──────────────────────────────────

struct GenState {
    /// Which conversation is being generated for
    conv_id: String,
    /// Tokens received so far
    pending_text: String,
    rx: mpsc::Receiver<StreamToken>,
    stop_flag: Arc<AtomicBool>,
}

// ─── App ───────────────────────────────────────────────

pub struct DesktopAI {
    config: Config,
    inference: Option<Arc<LlamaInference>>,
    current_conv: Conversation,

    // Chat
    input_text: String,
    gen: Option<GenState>,

    // Downloads (multiple concurrent)
    downloads: HashMap<String, DownloadState>,

    // Hardware info
    cpu_cores: usize,
    ram_warning: Option<String>,

    // API server
    api_server: Option<ApiServer>,

    // Search
    show_search_panel: bool,
    search_query: String,
    search_results: Vec<SearchResult>,
    search_loading: bool,
    search_error: Option<String>,
    search_rx: Option<mpsc::Receiver<Result<Vec<SearchResult>, String>>>,

    // UI
    show_model_select: bool,
    show_settings: bool,
    status_message: String,
    error_message: Option<String>,
    theme_applied: bool,
    confirm_action: Option<ConfirmAction>,
}

#[derive(Clone, PartialEq)]
enum ConfirmAction {
    DeleteAllModels,
    DeleteAllConversations,
    ResetApp,
    UninstallApp,
}

fn detect_hardware() -> (usize, Option<String>) {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);

    // Try to get total RAM via Windows API
    let ram_gb = get_total_ram_gb();
    let warning = if ram_gb > 0.0 && ram_gb < 4.0 {
        Some(format!("⚠ 检测到内存仅 {:.1} GB，建议只使用 0.5B 或 1.7B 模型。大模型会严重卡顿或无法加载。", ram_gb))
    } else if ram_gb > 0.0 && ram_gb < 8.0 {
        Some(format!("ℹ 检测到内存 {:.1} GB，可使用 3B 以下的模型。7B+ 模型需要 8GB 以上内存。", ram_gb))
    } else {
        None
    };
    (cores, warning)
}

#[cfg(windows)]
fn get_total_ram_gb() -> f64 {
    use std::mem;
    unsafe {
        let mut mem_status: windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX = mem::zeroed();
        mem_status.dwLength = mem::size_of::<windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX>() as u32;
        if windows_sys::Win32::System::SystemInformation::GlobalMemoryStatusEx(&mut mem_status) != 0 {
            mem_status.ullTotalPhys as f64 / (1024.0 * 1024.0 * 1024.0)
        } else {
            0.0
        }
    }
}

#[cfg(not(windows))]
fn get_total_ram_gb() -> f64 { 0.0 }

impl DesktopAI {
    pub fn new() -> Self {
        let config = config::load_config();
        let current_conv = if let Some(ref id) = config.last_conversation_id {
            Conversation::load(id).unwrap_or_else(Conversation::new)
        } else {
            Conversation::new()
        };

        let (cpu_cores, ram_warning) = detect_hardware();

        Self {
            config,
            inference: None,
            current_conv,
            input_text: String::new(),
            gen: None,
            downloads: HashMap::new(),
            cpu_cores,
            ram_warning,
            api_server: None,
            show_search_panel: false,
            search_query: String::new(),
            search_results: Vec::new(),
            search_loading: false,
            search_error: None,
            search_rx: None,
            show_model_select: false,
            show_settings: false,
            status_message: "就绪".into(),
            error_message: None,
            theme_applied: false,
            confirm_action: None,
        }
    }

    fn is_generating(&self) -> bool { self.gen.is_some() }

    fn load_selected_model(&mut self) {
        let model_id = match &self.config.selected_model_id {
            Some(id) => id.clone(),
            None => return,
        };
        let info = match find_model(&self.config.model_catalog, &model_id) {
            Some(i) => i.clone(),
            None => return,
        };
        let model_path = config::models_dir().join(&info.filename);
        if !model_path.exists() { return; }

        self.status_message = format!("加载 {}...", info.name);
        let n_ctx = self.config.n_ctx;
        let n_threads = if self.config.n_threads == "auto" {
            std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4)
        } else {
            self.config.n_threads.parse().unwrap_or(4)
        };

        let path_str = model_path.to_string_lossy().to_string();
        match LlamaInference::load(&path_str, n_ctx, n_threads) {
            Ok(inf) => {
                let inf = Arc::new(inf);
                // Start API server if enabled
                if self.config.api_enabled {
                    if let Some(ref mut old) = self.api_server { old.stop(); }
                    let port = self.config.api_port;
                    let server = ApiServer::start(Arc::clone(&inf), port, info.name.clone());
                    self.api_server = Some(server);
                    self.status_message = format!("{} 就绪 | API: http://127.0.0.1:{}/v1", info.name, port);
                } else {
                    self.status_message = format!("{} 就绪", info.name);
                }
                self.inference = Some(inf);
            }
            Err(e) => { self.status_message = format!("加载失败: {}", e); }
        }
    }

    // ─── Concurrent downloads ──────────────────────────

    fn start_download(&mut self, model_id: &str) {
        if self.downloads.contains_key(model_id) { return; }

        let info = match find_model(&self.config.model_catalog, model_id) {
            Some(i) => i.clone(),
            None => return,
        };

        let dest = config::models_dir().join(&info.filename);
        let url = info.url.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();

        self.downloads.insert(model_id.to_string(), DownloadState {
            progress: 0.0,
            status: "连接中...".into(),
            rx,
            cancel: cancel.clone(),
        });

        thread::spawn(move || {
            downloader::download_model(&url, dest, cancel, tx);
        });
    }

    fn cancel_download(&mut self, model_id: &str) {
        if let Some(ds) = self.downloads.get(model_id) {
            ds.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn poll_all_downloads(&mut self) {
        let mut finished = vec![];
        let mut errors = vec![];

        for (id, ds) in self.downloads.iter_mut() {
            while let Ok(msg) = ds.rx.try_recv() {
                match msg {
                    DownloadMsg::Progress { percent, downloaded_mb, total_mb } => {
                        ds.progress = percent as f32 / 100.0;
                        ds.status = format!("{:.0}% ({:.0}/{:.0}MB)", percent as f64, downloaded_mb, total_mb);
                    }
                    DownloadMsg::Status(s) => { ds.status = s; }
                    DownloadMsg::Done => { finished.push(id.clone()); break; }
                    DownloadMsg::Error(e) => { errors.push((id.clone(), e)); break; }
                }
            }
        }

        for id in finished {
            self.downloads.remove(&id);
            // Auto-select if this was the target model
            if self.config.selected_model_id.as_deref() == Some(&id) {
                self.load_selected_model();
            }
        }
        for (id, err) in errors {
            self.downloads.remove(&id);
            self.error_message = Some(format!("下载 {} 失败: {}", id, err));
        }
    }

    // ─── Generation (detached from conversation nav) ────

    fn send_message(&mut self) {
        if self.is_generating() { return; }
        let text = self.input_text.trim().to_string();
        if text.is_empty() { return; }
        self.input_text.clear();

        self.current_conv.add_message("user", &text);

        let inf = match self.inference.as_ref() {
            Some(inf) => Arc::clone(inf),
            None => {
                self.status_message = "模型未加载".into();
                return;
            }
        };

        let messages = self.current_conv.context_messages(
            Some(&self.config.system_prompt), 20
        );
        let conv_id = self.current_conv.id.clone();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();

        let stop = stop_flag.clone();

        self.gen = Some(GenState {
            conv_id,
            pending_text: String::new(),
            rx,
            stop_flag,
        });

        thread::spawn(move || {
            let prompt = inference::format_chatml(&messages);
            inference::run_inference(inf, prompt, stop, tx, 2048);
        });
    }

    fn stop_generation(&mut self) {
        if let Some(ref gen) = self.gen {
            gen.stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn poll_generation(&mut self) {
        let gen = match self.gen.as_mut() {
            Some(g) => g,
            None => return,
        };

        let mut done = false;
        while let Ok(token) = gen.rx.try_recv() {
            match token {
                StreamToken::Text(t) => gen.pending_text.push_str(&t),
                StreamToken::Done => { done = true; break; }
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

            // Save to the original conversation
            if let Some(mut conv) = Conversation::load(&conv_id) {
                conv.add_message("assistant", &response);
            }

            self.gen = None;
        }
    }

    // ─── Conversation management ────────────────────────

    fn new_conversation(&mut self) {
        self.current_conv = Conversation::new();
    }

    fn load_conversation(&mut self, id: &str) {
        if let Some(conv) = Conversation::load(id) {
            self.current_conv = conv;
            self.config.last_conversation_id = Some(id.to_string());
            config::save_config(&self.config);
        }
    }

    fn delete_conversation(&mut self, id: &str) {
        Conversation::delete(id);
        if self.current_conv.id == id {
            self.current_conv = Conversation::new();
        }
    }

    fn delete_model_file(&mut self, model_id: &str) {
        if let Some(info) = find_model(&self.config.model_catalog, model_id) {
            let path = config::models_dir().join(&info.filename);
            let _ = std::fs::remove_file(&path);
        }
    }

    fn delete_all_models(&mut self) {
        let dir = config::models_dir();
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        self.inference = None;
        self.config.selected_model_id = None;
        config::save_config(&self.config);
        self.status_message = "所有模型已删除".into();
    }

    fn delete_all_conversations(&mut self) {
        let dir = config::conversations_dir();
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        self.current_conv = Conversation::new();
        self.gen = None;
        self.status_message = "所有对话已删除".into();
    }

    fn reset_app(&mut self) {
        let app_dir = config::app_dirs().data_dir().to_path_buf();
        let _ = std::fs::remove_dir_all(&app_dir);
        let config_file = config::config_path();
        let _ = std::fs::remove_file(&config_file);
        self.inference = None;
        self.config = Config::default();
        self.current_conv = Conversation::new();
        self.gen = None;
        self.downloads.clear();
        self.show_settings = false;
        self.status_message = "应用已重置。请重新选择模型。".into();
    }

    fn uninstall_app(&mut self) {
        // Delete all user data first
        let app_dir = config::app_dirs().data_dir().to_path_buf();
        let _ = std::fs::remove_dir_all(&app_dir);
        let config_file = config::config_path();
        let _ = std::fs::remove_file(&config_file);

        // Schedule self-deletion via batch script
        let exe_path = std::env::current_exe().unwrap_or_default();
        let exe_dir = exe_path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let batch_path = exe_dir.join("_uninstall.bat");
        if let Ok(mut f) = std::fs::File::create(&batch_path) {
            use std::io::Write;
            let _ = writeln!(f, "@echo off");
            let _ = writeln!(f, "echo 桌面AI 卸载中...");
            let _ = writeln!(f, "timeout /t 2 /nobreak >nul");
            let _ = writeln!(f, "del /f /q \"{}\"", exe_path.display());
            let _ = writeln!(f, "del /f /q \"{}\"", exe_dir.join("llama.dll").display());
            let _ = writeln!(f, "del /f /q \"{}\"", batch_path.display());
            let _ = writeln!(f, "echo 桌面AI 已卸载。");
            let _ = writeln!(f, "timeout /t 2 /nobreak >nul");
        }

        // Launch the batch script detached and exit
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "/MIN", ""])
            .arg(batch_path.to_string_lossy().to_string())
            .spawn();

        std::process::exit(0);
    }

    fn start_search(&mut self) {
        let query = self.search_query.trim().to_string();
        if query.is_empty() || self.search_loading { return; }
        self.search_loading = true;
        self.search_error = None;
        self.search_results.clear();
        let (tx, rx) = mpsc::channel();
        self.search_rx = Some(rx);
        thread::spawn(move || {
            let _ = tx.send(search::search_duckduckgo(&query));
        });
    }

    fn poll_search(&mut self) {
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
        self.poll_all_downloads();
        self.poll_generation();
        self.poll_search();

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

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let icon = if self.config.theme == "dark" { "☀" } else { "🌙" };
                    if ui.button(icon).clicked() {
                        self.config.theme = if self.config.theme == "dark" { "light".into() } else { "dark".into() };
                        apply_theme(ctx, &self.config.theme);
                        config::save_config(&self.config);
                    }
                    if ui.button("⚙").clicked() { self.show_settings = true; }
                });
            });

            // Show active downloads
            if !self.downloads.is_empty() {
                ui.separator();
                let mut to_cancel = vec![];
                for (id, ds) in self.downloads.iter() {
                    if let Some(info) = find_model(&self.config.model_catalog, id) {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&info.name).size(11.0));
                            ui.add(egui::ProgressBar::new(ds.progress)
                                .desired_width(150.0)
                                .text(&ds.status));
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
                ui.heading("桌面AI");
                ui.label(RichText::new("v5.1").size(10.0).color(Color32::GRAY));
                ui.add_space(8.0);

                if ui.button("+ 新对话").clicked() {
                    self.new_conversation();
                }

                ui.add_space(4.0);
                ui.separator();
                ui.label(RichText::new("对话历史").size(11.0).color(Color32::GRAY));

                ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
                    let convs = Conversation::list_all();
                    for conv in &convs {
                        ui.horizontal(|ui| {
                            let title = if conv.title.len() > 18 {
                                format!("{}...", &conv.title[..18])
                            } else { conv.title.clone() };
                            let active = conv.id == self.current_conv.id;
                            if ui.selectable_label(active, &title).clicked() {
                                self.load_conversation(&conv.id);
                            }
                            if ui.button("✕").clicked() {
                                self.delete_conversation(&conv.id);
                            }
                        });
                        ui.label(RichText::new(format!("{} 条消息", conv.message_count))
                            .size(10.0).color(Color32::GRAY));
                    }
                });

                ui.add_space(8.0);
                ui.separator();
                if ui.button("切换模型").clicked() { self.show_model_select = true; }
                if ui.button("搜索").clicked() { self.show_search_panel = !self.show_search_panel; }
            });

        // ─── Chat area ─────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false; 2]).show(ui, |ui| {
                let font_size = self.config.font_size as f32;

                for msg in &self.current_conv.messages {
                    let is_user = msg.role == "user";
                    if is_user {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            egui::Frame::default()
                                .fill(Color32::from_rgb(13, 110, 253))
                                .corner_radius(12)
                                .inner_margin(egui::Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    ui.label(RichText::new(&msg.content).size(font_size).color(Color32::WHITE));
                                });
                        });
                    } else {
                        let bg = if self.config.theme == "dark" { Color32::from_rgb(45, 45, 45) }
                                 else { Color32::from_rgb(232, 232, 232) };
                        egui::Frame::default().fill(bg).corner_radius(12)
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                markdown::render_markdown(ui, &msg.content, font_size);
                            });
                    }
                    ui.add_space(4.0);
                }

                // Streaming text
                if let Some(ref gen) = self.gen {
                    if gen.conv_id == self.current_conv.id && !gen.pending_text.is_empty() {
                        let bg = if self.config.theme == "dark" { Color32::from_rgb(45, 45, 45) }
                                 else { Color32::from_rgb(232, 232, 232) };
                        egui::Frame::default().fill(bg).corner_radius(12)
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.label(RichText::new(&gen.pending_text).size(font_size));
                                let blink = ctx.input(|i| i.time) as u64 % 1000 < 500;
                                ui.label(RichText::new(" ▌").color(
                                    if blink { Color32::WHITE } else { Color32::TRANSPARENT }
                                ));
                            });
                    }
                }

                // Generating indicator for other conversation
                if let Some(ref gen) = self.gen {
                    if gen.conv_id != self.current_conv.id {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.label(RichText::new("⏳ 另一个对话正在生成回复...")
                                .size(13.0).color(Color32::GRAY));
                        });
                    }
                }

                // Welcome
                if self.current_conv.messages.is_empty() && !self.is_generating() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(80.0);
                        ui.label(RichText::new("欢迎使用 桌面AI").size(18.0).strong());
                        ui.add_space(8.0);
                        ui.label("选择模型后即可开始本地 AI 对话");
                        ui.label(RichText::new("支持同时下载多个模型").size(12.0).color(Color32::GRAY));
                    });
                }
            });
        });

        // ─── Input bar ─────────────────────────────
        egui::TopBottomPanel::bottom("input").show(ctx, |ui| {
            let can_send = !self.is_generating() && self.inference.is_some();
            let is_gen = self.is_generating();
            ui.horizontal(|ui| {
                let hint = if is_gen { "等待生成完成..." }
                           else if self.inference.is_some() { "输入消息... (Enter 发送)" }
                           else { "请先加载模型" };
                ui.add_sized(
                    vec2(ui.available_width() - 80.0, 50.0),
                    TextEdit::multiline(&mut self.input_text)
                        .hint_text(hint)
                        .desired_rows(2),
                );

                if is_gen {
                    if ui.add_sized(vec2(70.0, 50.0), egui::Button::new(
                        RichText::new("停止").size(14.0).color(Color32::WHITE)
                    ).fill(Color32::from_rgb(192, 57, 43))).clicked() {
                        self.stop_generation();
                    }
                } else {
                    let btn = ui.add_sized(vec2(70.0, 50.0), egui::Button::new(
                        RichText::new("发送").size(14.0)
                    ));
                    if (btn.clicked() || (ui.input(|i| i.key_pressed(egui::Key::Enter))))
                        && can_send && !self.input_text.trim().is_empty()
                    {
                        self.send_message();
                    }
                }
            });
        });

        // ─── Model select window ───────────────────
        if self.show_model_select {
            egui::Window::new("选择模型")
                .collapsible(false).resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(RichText::new("选择一个模型（可同时下载多个）").size(14.0).strong());
                    ui.add_space(4.0);

                    // Hardware info
                    ui.label(RichText::new(format!("你的设备: {} 核 CPU", self.cpu_cores))
                        .size(11.0).color(Color32::GRAY));
                    if let Some(ref warn) = self.ram_warning {
                        ui.label(RichText::new(warn).size(11.0).color(Color32::from_rgb(255, 200, 50)));
                    }
                    ui.separator();
                    ui.add_space(4.0);

                    for model in &self.config.model_catalog.clone() {
                        let downloaded = config::models_dir().join(&model.filename).exists();
                        let is_downloading = self.downloads.contains_key(&model.id);
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&model.name).size(14.0).strong());
                                for tag in &model.tags {
                                    ui.label(RichText::new(tag).size(10.0)
                                        .background_color(Color32::from_rgb(31, 106, 165))
                                        .color(Color32::WHITE));
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if is_downloading {
                                        if let Some(ds) = self.downloads.get(&model.id) {
                                            ui.label(&ds.status);
                                            ui.add(egui::ProgressBar::new(ds.progress).desired_width(100.0));
                                            if ui.button("取消").clicked() {
                                                self.cancel_download(&model.id);
                                            }
                                        }
                                    } else if downloaded {
                                        ui.label(RichText::new("✓ 已下载")
                                            .color(Color32::from_rgb(76, 175, 80)));
                                        if ui.button("使用").clicked() {
                                            self.config.selected_model_id = Some(model.id.clone());
                                            config::save_config(&self.config);
                                            self.load_selected_model();
                                            self.show_model_select = false;
                                        }
                                    } else {
                                        if ui.button("下载").clicked() {
                                            self.start_download(&model.id);
                                        }
                                    }
                                });
                            });
                            ui.label(RichText::new(&model.desc).size(11.0).color(Color32::GRAY));
                            ui.label(RichText::new(format!("约 {:.2} GB", model.size_gb))
                                .size(11.0).color(Color32::from_rgb(76, 175, 80)));

                            let ram_gb = get_total_ram_gb();
                            let rec_ram = model.size_gb * 3.0 + 1.0; // rough: model × 3 + 1GB overhead
                            if ram_gb > 0.0 && ram_gb < rec_ram {
                                ui.label(RichText::new(
                                    format!("⚠ 推荐 {:.0} GB 内存，你的设备可能不足", rec_ram)
                                ).size(10.0).color(Color32::from_rgb(255, 165, 0)));
                            }
                        });
                    }
                    if ui.button("关闭").clicked() { self.show_model_select = false; }
                });
        }

        // ─── Search panel ──────────────────────────
        if self.show_search_panel {
            egui::Window::new("搜索")
                .collapsible(false).resizable(true)
                .anchor(egui::Align2::RIGHT_TOP, [-10.0, 30.0])
                .default_width(350.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_sized(vec2(ui.available_width() - 60.0, 24.0),
                            TextEdit::singleline(&mut self.search_query)
                                .hint_text("输入搜索关键词..."));
                        if (ui.button("搜索").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)))
                            && !self.search_loading
                        {
                            self.start_search();
                        }
                    });

                    if self.search_loading {
                        ui.add_space(4.0);
                        ui.label(RichText::new("搜索中...").color(Color32::GRAY));
                    }

                    if let Some(ref err) = self.search_error {
                        ui.label(RichText::new(err).color(Color32::from_rgb(255, 80, 80)));
                    }

                    if !self.search_results.is_empty() {
                        ui.separator();
                        ui.label(RichText::new(format!("{} 条结果", self.search_results.len()))
                            .size(11.0).color(Color32::GRAY));
                        ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                            for result in &self.search_results {
                                ui.group(|ui| {
                                    ui.label(RichText::new(&result.title).size(12.0).strong());
                                    if !result.url.is_empty() {
                                        ui.label(RichText::new(&result.url).size(10.0)
                                            .color(Color32::from_rgb(100, 180, 255)));
                                    }
                                    if !result.snippet.is_empty() {
                                        ui.label(RichText::new(&result.snippet).size(11.0));
                                    }
                                });
                                ui.add_space(3.0);
                            }
                        });
                    }
                });
        }

        // ─── Settings ─────────────────────────────
        if self.show_settings {
            egui::Window::new("设置")
                .collapsible(false).resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ScrollArea::vertical().max_height(500.0).show(ui, |ui| {

                    ui.label(RichText::new("主题外观").strong());
                    ui.horizontal(|ui| {
                        if ui.selectable_value(&mut self.config.theme, "dark".into(), "深色").clicked() {
                            apply_theme(ctx, &self.config.theme);
                        }
                        if ui.selectable_value(&mut self.config.theme, "light".into(), "浅色").clicked() {
                            apply_theme(ctx, &self.config.theme);
                        }
                    });
                    ui.add_space(8.0);

                    ui.label(RichText::new("字号").strong());
                    ui.add(egui::Slider::new(&mut self.config.font_size, 10..=24).text("pt"));
                    ui.add_space(8.0);

                    ui.label(RichText::new("上下文长度").strong());
                    ui.add(egui::Slider::new(&mut self.config.n_ctx, 512..=8192).text("tokens"));
                    ui.add_space(8.0);

                    ui.label(RichText::new("CPU 线程数").strong());
                    let mut thr = self.config.n_threads.clone();
                    egui::ComboBox::from_id_salt("threads")
                        .selected_text(&thr)
                        .show_ui(ui, |ui| {
                            for opt in &["auto", "2", "4", "6", "8", "12", "16"] {
                                ui.selectable_value(&mut thr, opt.to_string(), *opt);
                            }
                        });
                    self.config.n_threads = thr;
                    ui.add_space(8.0);

                    ui.label(RichText::new("系统提示词").strong());
                    ui.add(TextEdit::multiline(&mut self.config.system_prompt)
                        .desired_rows(2)
                        .hint_text("You are a helpful assistant."));
                    ui.add_space(8.0);

                    // API Server
                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(RichText::new("API 服务").size(13.0).strong());
                    ui.checkbox(&mut self.config.api_enabled, "启用本地 API 服务 (OpenAI 兼容)");
                    if self.config.api_enabled {
                        ui.horizontal(|ui| {
                            ui.label("端口:");
                            let mut port_str = self.config.api_port.to_string();
                            if ui.add_sized(vec2(80.0, 20.0), TextEdit::singleline(&mut port_str)).changed() {
                                if let Ok(p) = port_str.parse() { self.config.api_port = p; }
                            }
                        });
                        ui.label(RichText::new(
                            format!("API 地址: http://127.0.0.1:{}/v1/chat/completions", self.config.api_port)
                        ).size(10.0).color(Color32::from_rgb(100, 180, 255)));
                        ui.label(RichText::new("支持 POST JSON, 兼容 OpenAI chat completions 格式")
                            .size(10.0).color(Color32::GRAY));
                    }
                    ui.add_space(8.0);

                    // Search engine
                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(RichText::new("搜索引擎").size(13.0).strong());
                    ui.checkbox(&mut self.config.search_enabled, "启用 DuckDuckGo 搜索");
                    ui.label(RichText::new("搜索按钮在左侧边栏底部").size(10.0).color(Color32::GRAY));
                    ui.add_space(8.0);

                    // Current model info
                    ui.separator();
                    ui.add_space(4.0);
                    if let Some(ref sel) = self.config.selected_model_id {
                        if let Some(info) = find_model(&self.config.model_catalog, sel) {
                            ui.label(RichText::new(format!("当前模型: {}", info.name))
                                .size(12.0).color(Color32::from_rgb(76, 175, 80)));
                        }
                    }
                    if ui.button("切换模型 (打开模型选择窗口)").clicked() {
                        self.show_settings = false;
                        self.show_model_select = true;
                    }
                    ui.add_space(8.0);

                    // Downloaded models
                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(RichText::new("已下载的模型").size(13.0).strong());
                    ui.add_space(2.0);
                    let models_dir = config::models_dir();
                    let downloaded_list: Vec<(String, String, f64)> = self.config.model_catalog.iter()
                        .filter_map(|m| {
                            let path = models_dir.join(&m.filename);
                            if path.exists() {
                                let size_mb = std::fs::metadata(&path)
                                    .map(|m| m.len() as f64 / 1_048_576.0).unwrap_or(0.0);
                                Some((m.id.clone(), m.name.clone(), size_mb))
                            } else { None }
                        })
                        .collect();
                    if downloaded_list.is_empty() {
                        ui.label(RichText::new("暂无已下载的模型")
                            .size(11.0).color(Color32::GRAY));
                    } else {
                        for (id, name, size_mb) in &downloaded_list {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("{}  ({:.0} MB)", name, size_mb))
                                    .size(11.0));
                                let model_id = id.clone();
                                let del_btn = egui::Button::new(
                                    RichText::new("删除").size(11.0).color(Color32::WHITE)
                                ).fill(Color32::from_rgb(180, 60, 60));
                                if ui.add_sized(vec2(40.0, 20.0), del_btn).clicked() {
                                    self.delete_model_file(&model_id);
                                }
                            });
                        }
                    }
                    ui.add_space(8.0);

                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(RichText::new("数据管理").size(13.0).strong());
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let del_models_btn = egui::Button::new(
                            RichText::new("删除所有模型").color(Color32::WHITE).size(12.0)
                        ).fill(Color32::from_rgb(180, 100, 60));
                        if ui.add(del_models_btn).clicked() {
                            self.confirm_action = Some(ConfirmAction::DeleteAllModels);
                        }
                        let del_convs_btn = egui::Button::new(
                            RichText::new("删除所有对话").color(Color32::WHITE).size(12.0)
                        ).fill(Color32::from_rgb(180, 100, 60));
                        if ui.add(del_convs_btn).clicked() {
                            self.confirm_action = Some(ConfirmAction::DeleteAllConversations);
                        }
                    });
                    ui.add_space(12.0);

                    ui.separator();
                    ui.label(RichText::new("⚠ 危险操作").size(12.0).color(Color32::from_rgb(255, 80, 80)));
                    ui.add_space(4.0);
                    let reset_btn = egui::Button::new(
                        RichText::new("重置应用 (删除全部数据)")
                            .color(Color32::WHITE)
                            .size(13.0)
                    ).fill(Color32::from_rgb(192, 57, 43))
                     .min_size(vec2(ui.available_width(), 28.0));
                    if ui.add(reset_btn).clicked() {
                        self.confirm_action = Some(ConfirmAction::ResetApp);
                    }
                    ui.add_space(6.0);
                    let uninstall_btn = egui::Button::new(
                        RichText::new("卸载应用 (删除程序及全部数据)")
                            .color(Color32::WHITE)
                            .size(13.0)
                    ).fill(Color32::from_rgb(160, 30, 30))
                     .min_size(vec2(ui.available_width(), 28.0));
                    if ui.add(uninstall_btn).clicked() {
                        self.confirm_action = Some(ConfirmAction::UninstallApp);
                    }
                    ui.add_space(12.0);

                    if ui.button("保存并关闭").clicked() {
                        config::save_config(&self.config);
                        self.show_settings = false;
                    }

                    }); // ScrollArea
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
                .collapsible(false).resizable(false)
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
                                ConfirmAction::UninstallApp => ("确定卸载", ConfirmAction::UninstallApp),
                                _ => ("确定删除", ConfirmAction::ResetApp),
                            };
                            let confirm_btn = egui::Button::new(
                                RichText::new(btn_text).color(Color32::WHITE)
                            ).fill(Color32::from_rgb(192, 57, 43));
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
                                    ConfirmAction::DeleteAllConversations => self.delete_all_conversations(),
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

        // ─── Error toast ───────────────────────────
        if let Some(ref err) = self.error_message.clone() {
            egui::Window::new("错误").collapsible(false).resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(err);
                    if ui.button("确定").clicked() { self.error_message = None; }
                });
        }

        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}
