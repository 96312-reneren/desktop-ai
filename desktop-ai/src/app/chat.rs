// DesktopAI sub-module: chat area + input bar
use super::DesktopAI;
use crate::config;
use crate::markdown;
use egui::{vec2, Color32, Label, RichText, ScrollArea, TextEdit};

impl DesktopAI {
    pub(crate) fn render_chat_area(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
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
                                    ui.add(
                                        Label::new(
                                            RichText::new(&msg.content)
                                                .size(font_size)
                                                .color(Color32::WHITE),
                                        )
                                        .selectable(true),
                                    );
                                });
                        });
                    } else {
                        let bg = if self.config.theme == "dark" {
                            Color32::from_rgb(45, 45, 45)
                        } else {
                            Color32::from_rgb(232, 232, 232)
                        };
                        egui::Frame::default()
                            .fill(bg)
                            .corner_radius(12)
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                markdown::render_markdown(ui, &msg.content, font_size);
                            });
                    }
                    ui.add_space(4.0);
                }

                if let Some(ref gen) = self.gen {
                    if gen.conv_id == self.current_conv.id && !gen.pending_text.is_empty() {
                        let bg = if self.config.theme == "dark" {
                            Color32::from_rgb(45, 45, 45)
                        } else {
                            Color32::from_rgb(232, 232, 232)
                        };
                        egui::Frame::default()
                            .fill(bg)
                            .corner_radius(12)
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.add(
                                    Label::new(RichText::new(&gen.pending_text).size(font_size))
                                        .selectable(true),
                                );
                                let blink = ctx.input(|i| i.time) as u64 % 1000 < 500;
                                ui.label(RichText::new(" ▌").color(if blink {
                                    Color32::WHITE
                                } else {
                                    Color32::TRANSPARENT
                                }));
                            });
                    }
                }

                if let Some(ref gen) = self.gen {
                    if gen.conv_id != self.current_conv.id {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.label(
                                RichText::new("⏳ 另一个对话正在生成回复...")
                                    .size(13.0)
                                    .color(Color32::GRAY),
                            );
                        });
                    }
                }

                if self.current_conv.messages.is_empty() && !self.is_generating() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(80.0);
                        ui.label(RichText::new("欢迎使用 桌面AI").size(18.0).strong());
                        ui.add_space(8.0);
                        ui.label("选择模型后即可开始本地 AI 对话");
                        ui.label(
                            RichText::new("支持同时下载多个模型")
                                .size(12.0)
                                .color(Color32::GRAY),
                        );
                    });
                }
            });
    }

    pub(crate) fn render_input_bar(&mut self, ui: &mut egui::Ui) {
        let can_send = !self.is_generating() && self.inference.is_some();
        let is_gen = self.is_generating();
        let input_empty = self.input_text.trim().is_empty();

        ui.horizontal(|ui| {
            let hint = if is_gen {
                "等待生成完成..."
            } else if self.inference.is_some() {
                "输入消息... (Ctrl+Enter 发送，最多1500字)"
            } else {
                "请先加载模型"
            };
            let before = self.input_text.chars().count();
            ui.add_sized(
                vec2(ui.available_width() - 80.0, 50.0),
                TextEdit::multiline(&mut self.input_text)
                    .hint_text(hint)
                    .char_limit(config::MAX_INPUT_GRAPHEMES)
                    .desired_rows(2),
            );

            if is_gen {
                if ui
                    .add_sized(
                        vec2(70.0, 50.0),
                        egui::Button::new(RichText::new("停止").size(14.0).color(Color32::WHITE))
                            .fill(Color32::from_rgb(192, 57, 43)),
                    )
                    .clicked()
                {
                    self.stop_generation();
                }
            } else if can_send && !input_empty {
                let btn = ui.add_sized(
                    vec2(70.0, 50.0),
                    egui::Button::new(RichText::new("发送").size(14.0)),
                );
                let ctrl_enter = ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.ctrl);
                if btn.clicked() || ctrl_enter {
                    let cleaned = config::strip_zero_width(self.input_text.trim());
                    if !cleaned.is_empty() {
                        self.input_text = cleaned;
                        self.send_message();
                    }
                }
            } else {
                // Visually disabled send button
                let _ = ui.add_enabled(
                    false,
                    egui::Button::new(RichText::new("发送").size(14.0)).min_size(vec2(70.0, 50.0)),
                );
            }

            // Track truncation for the hint below
            if before >= config::MAX_INPUT_GRAPHEMES
                && self.input_text.chars().count() >= config::MAX_INPUT_GRAPHEMES
            {
                // Store hint state — the label is rendered after the horizontal block
                // by checking the char count again in the calling code.
            }
        });

        // ── Truncation hint (below the input bar) ──
        if self.input_text.chars().count() >= config::MAX_INPUT_GRAPHEMES
            && !self.input_text.is_empty()
        {
            ui.label(
                RichText::new(format!("已自动截断至 {} 字符", config::MAX_INPUT_GRAPHEMES,))
                    .size(10.0)
                    .color(Color32::from_rgb(76, 175, 80)),
            );
        }
    }
}
