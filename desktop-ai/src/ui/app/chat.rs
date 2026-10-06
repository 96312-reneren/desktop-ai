// DesktopAI sub-module: chat area + input bar (Material Design 3 styling)
use super::{theme, DesktopAI};
use crate::store::config;
use crate::ui::markdown;
use egui::{vec2, Color32, CornerRadius, Label, Margin, RichText, ScrollArea, Stroke, TextEdit};

impl DesktopAI {
    pub(crate) fn render_chat_area(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let m = theme::palette(self.config.theme != "light");
        ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let font_size = self.config.font_size as f32;

                for msg in &self.current_conv.messages {
                    let is_user = msg.role == "user";
                    if is_user {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            theme::user_bubble(m).show(ui, |ui| {
                                ui.add(
                                    Label::new(
                                        RichText::new(&msg.content)
                                            .size(font_size)
                                            .color(m.on_primary),
                                    )
                                    .selectable(true),
                                );
                            });
                        });
                    } else {
                        theme::bot_bubble(m).show(ui, |ui| {
                            markdown::render_markdown(ui, &msg.content, font_size);
                        });
                    }
                    ui.add_space(4.0);
                }

                if let Some(ref gen) = self.gen {
                    if gen.conv_id == self.current_conv.id {
                        // Context-budget notice (truncation / degraded context).
                        if let Some(ref notice) = gen.notice {
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new(format!("ℹ {notice}"))
                                        .size(11.5)
                                        .color(m.on_surface_variant),
                                );
                            });
                            ui.add_space(2.0);
                        }
                        if !gen.pending_text.is_empty() {
                            theme::bot_bubble(m).show(ui, |ui| {
                                ui.add(
                                    Label::new(RichText::new(&gen.pending_text).size(font_size))
                                        .selectable(true),
                                );
                                let blink = ctx.input(|i| i.time) as u64 % 1000 < 500;
                                ui.label(RichText::new(" ▌").color(if blink {
                                    m.on_surface
                                } else {
                                    Color32::TRANSPARENT
                                }));
                            });
                        }
                    }
                }

                if let Some(ref gen) = self.gen {
                    if gen.conv_id != self.current_conv.id {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.label(
                                RichText::new("⏳ 另一个对话正在生成回复...")
                                    .size(13.0)
                                    .color(m.on_surface_variant),
                            );
                        });
                    }
                }

                if self.current_conv.messages.is_empty() && !self.is_generating() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(80.0);
                        ui.label(RichText::new("✦ 桌面AI").size(26.0).color(m.primary));
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("本地大模型推理 · 数据不出本机")
                                .size(14.0)
                                .color(m.on_surface_variant),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("选择模型后即可开始对话")
                                .size(12.0)
                                .color(m.outline),
                        );
                    });
                }
            });
    }

    pub(crate) fn render_input_bar(&mut self, ui: &mut egui::Ui) {
        let m = theme::palette(self.config.theme != "light");
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

            // M3 outlined text field: rounded container that highlights on focus.
            let focused = ui.memory(|mem| mem.has_focus(egui::Id::new("chat_input")));
            let stroke = if focused {
                Stroke::new(1.5_f32, m.primary)
            } else {
                Stroke::new(1.0_f32, m.outline_variant)
            };
            let btn_w = 76.0;
            egui::Frame::new()
                .fill(m.surface_container_lowest)
                .stroke(stroke)
                .corner_radius(CornerRadius::same(24))
                .inner_margin(Margin {
                    left: 16,
                    right: 8,
                    top: 4,
                    bottom: 4,
                })
                .show(ui, |ui| {
                    ui.add_sized(
                        vec2(ui.available_width() - btn_w - 12.0, 44.0),
                        TextEdit::multiline(&mut self.input_text)
                            .id(egui::Id::new("chat_input"))
                            .hint_text(hint)
                            .char_limit(config::MAX_INPUT_GRAPHEMES)
                            .desired_rows(1)
                            .frame(false),
                    );
                });

            if is_gen {
                if ui
                    .add_sized(
                        vec2(btn_w, 44.0),
                        theme::error_button(RichText::new("■ 停止").size(14.0), m),
                    )
                    .clicked()
                {
                    self.stop_generation();
                }
            } else if can_send && !input_empty {
                let btn = ui.add_sized(
                    vec2(btn_w, 44.0),
                    theme::primary_button(RichText::new("➤ 发送").size(14.0), m),
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
                let _ = ui.add_enabled(
                    false,
                    egui::Button::new(RichText::new("➤ 发送").size(14.0))
                        .min_size(vec2(btn_w, 44.0))
                        .corner_radius(CornerRadius::same(22)),
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
                    .color(m.success),
            );
        }
    }
}
