// DesktopAI eframe::App implementation: the update() entry point that
// orchestrates polling, panel layout, window management, and dialog rendering.
//
// All business logic lives in handlers.rs; this file is purely about
// coordinating the egui frame.

use super::state::{apply_theme, DesktopAI, StartupNotice};
use crate::llm::model_catalog::find_model;
use crate::store::config;
use eframe::egui;
use egui::{Color32, RichText};
use std::time::Duration;

/// How long a toast notification stays visible before auto-dismissing.
const TOAST_DURATION_SECS: u64 = 5;
/// Sidebar width when expanded.
const SIDEBAR_WIDTH: f32 = 200.0;
/// Collapsed sidebar width (just the toggle button).
const SIDEBAR_COLLAPSED_WIDTH: f32 = 36.0;
/// Auto-collapse the sidebar when the window is narrower than this.
const NARROW_WINDOW_THRESHOLD: f32 = 700.0;

impl eframe::App for DesktopAI {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_model_load();
        self.poll_all_downloads();
        self.poll_generation();
        self.poll_search();
        self.poll_kb_job();
        self.poll_catalog_refresh();

        // Auto-dismiss expired toast
        if let Some((_, created)) = &self.toast {
            if created.elapsed() > Duration::from_secs(TOAST_DURATION_SECS) {
                self.toast = None;
            }
        }

        // Auto-collapse sidebar on very narrow windows (#12)
        let window_w = ctx.screen_rect().width();
        if window_w < NARROW_WINDOW_THRESHOLD && !self.config.sidebar_collapsed {
            self.config.sidebar_collapsed = true;
            // Don't save config here — this is a transient responsive state
        }

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
            } else if self.is_generating() {
                // #11: Esc stops generation when no dialog is open
                self.stop_generation();
            }
        }

        // Apply saved theme only once on startup
        if !self.theme_applied {
            self.theme_applied = true;
            apply_theme(ctx, &self.config.theme);
        }

        // ─── 完整性告警横幅（红队缓解：ffi SHA 基线失配抬升到 UI）──
        if crate::llm::ffi::integrity_alert_triggered() {
            egui::TopBottomPanel::top("integrity_banner").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("⚠")
                            .size(16.0)
                            .color(Color32::from_rgb(244, 67, 54)),
                    );
                    ui.label(
                        RichText::new(
                            "模型库完整性告警：llama 运行库的 SHA-256 与上次记录不一致，可能已被替换或升级。",
                        )
                        .color(Color32::from_rgb(244, 67, 54)),
                    );
                    ui.label("基线已自动刷新；如非本人操作，请重新下载运行库。");
                });
            });
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

        // ─── Sidebar (collapsible, persisted) ──────
        if self.config.sidebar_collapsed {
            // Collapsed: just a narrow strip with an expand button
            egui::SidePanel::left("sidebar_collapsed")
                .resizable(false)
                .exact_width(SIDEBAR_COLLAPSED_WIDTH)
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(8.0);
                        if ui
                            .button(RichText::new("»").size(16.0))
                            .on_hover_text("展开侧边栏")
                            .clicked()
                        {
                            self.config.sidebar_collapsed = false;
                            config::save_config(&self.config);
                        }
                    });
                });
        } else {
            egui::SidePanel::left("sidebar")
                .resizable(false)
                .default_width(SIDEBAR_WIDTH)
                .show(ctx, |ui| {
                    // Collapse button at the very top
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            if ui
                                .button(RichText::new("«").size(12.0))
                                .on_hover_text("折叠侧边栏")
                                .clicked()
                            {
                                self.config.sidebar_collapsed = true;
                                config::save_config(&self.config);
                            }
                        });
                    });
                    self.render_sidebar(ui);
                });
        }

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
            // Fixed size relative to the main window (clamped): the model
            // list scrolls inside, so the window never grows off-screen.
            let w = (ctx.screen_rect().width() * 0.55).clamp(460.0, 720.0);
            let h = (ctx.screen_rect().height() * 0.75).clamp(420.0, 760.0);
            let mut open = self.show_model_select;
            egui::Window::new("选择模型")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .fixed_size([w, h])
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    self.render_model_select(ui);
                });
            self.show_model_select &= open;
        }

        // ─── Search panel ──────────────────────────
        if self.show_search_panel {
            let w = (ctx.screen_rect().width() * 0.32).clamp(320.0, 460.0);
            let h = (ctx.screen_rect().height() * 0.72).clamp(380.0, 720.0);
            let mut open = self.show_search_panel;
            egui::Window::new("搜索")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .fixed_size([w, h])
                .anchor(egui::Align2::RIGHT_TOP, [-12.0, 36.0])
                .show(ctx, |ui| {
                    self.render_search_panel(ui);
                });
            self.show_search_panel &= open;
        }

        // ─── Knowledge Base panel ────────────────────
        if self.show_kb_panel {
            let w = (ctx.screen_rect().width() * 0.34).clamp(340.0, 500.0);
            let h = (ctx.screen_rect().height() * 0.78).clamp(420.0, 780.0);
            let mut open = self.show_kb_panel;
            egui::Window::new("知识库")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .fixed_size([w, h])
                .anchor(egui::Align2::RIGHT_TOP, [-12.0, 36.0])
                .show(ctx, |ui| {
                    self.render_kb_panel(ui);
                });
            self.show_kb_panel &= open;
        }

        // ─── Settings ─────────────────────────────
        if self.show_settings {
            let w = (ctx.screen_rect().width() * 0.45).clamp(420.0, 640.0);
            let h = (ctx.screen_rect().height() * 0.75).clamp(420.0, 780.0);
            let mut open = self.show_settings;
            egui::Window::new("设置")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .fixed_size([w, h])
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    self.render_settings(ctx, ui);
                });
            self.show_settings &= open;
        }

        // ─── Confirm dialog ───────────────────────
        self.render_confirm_dialog(ctx);

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
                            match crate::platform::shortcut::create_desktop_shortcut() {
                                Ok(true) => {
                                    self.status_message = "桌面快捷方式已创建".into();
                                }
                                Ok(false) => {
                                    self.status_message = "未创建快捷方式".into();
                                }
                                Err(e) => {
                                    self.show_toast(format!("创建快捷方式失败: {}", e));
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

        // ─── Error handling: legacy modal dialog for error_message,
        // plus non-modal toast for toast notifications ──
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

        // ─── Toast notification (non-modal, auto-dismissing) ──
        if let Some((ref msg, _)) = self.toast.clone() {
            let m = super::theme::palette(self.config.theme != "light");
            egui::Area::new(egui::Id::new("toast_area"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -60.0])
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(m.surface_container_high)
                        .stroke(egui::Stroke::new(1.0, m.outline_variant))
                        .corner_radius(egui::CornerRadius::same(12))
                        .inner_margin(egui::Margin::symmetric(20, 12))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("ℹ").size(14.0).color(m.primary));
                                ui.label(RichText::new(msg).size(13.0).color(m.on_surface));
                                if ui.small_button("✕").clicked() {
                                    self.toast = None;
                                }
                            });
                        });
                });
        }

        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}
