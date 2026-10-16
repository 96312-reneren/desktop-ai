// DesktopAI sub-module: chat area + input bar (Material Design 3 styling)
use super::{theme, DesktopAI};
use crate::store::config;
use crate::ui::markdown;
use egui::{vec2, Color32, CornerRadius, Label, Margin, RichText, ScrollArea, Stroke, TextEdit};

/// Rough height estimate for an unmeasured message.
/// Uses content length and role to guess how tall the bubble will be.
/// After the message is rendered once, the actual height is cached in
/// `DesktopAI::msg_heights` and this estimate is no longer consulted.
fn estimate_msg_height(content: &str, role: &str, font_size: f32, avail_width: f32) -> f32 {
    let chars = content.chars().count();
    if chars == 0 {
        return 36.0;
    }
    // Average char width: CJK ≈ 1.0×font_size, Latin ≈ 0.55×font_size.
    // Use 0.7 as a conservative blend.
    let avg_char_w = font_size * 0.7;
    let effective_w = if role == "user" {
        // user bubble is right-aligned and narrower
        (avail_width * 0.65 - 32.0).max(60.0)
    } else {
        (avail_width - 80.0).max(100.0)
    };
    let chars_per_line = (effective_w / avg_char_w).max(1.0);
    let lines = (chars as f32 / chars_per_line).ceil();
    let line_h = font_size * 1.5;
    let text_h = lines * line_h;
    // Bubble padding + code-block extra
    let pad: f32 = if role == "user" { 24.0 } else { 28.0 };
    let code_extra = if content.contains("```") {
        // Each code block adds ~40px for the frame + button row
        content.matches("```").count() as f32 * 20.0 + 30.0
    } else {
        0.0
    };
    text_h + pad + code_extra
}

/// Spacing between consecutive message bubbles.
const MSG_SPACING: f32 = 4.0;
/// How many extra messages to render above/below the visible viewport
/// to avoid pop-in during fast scrolling.
const SCROLL_MARGIN: usize = 4;
/// Invalidate the height cache when the window width changes by more
/// than this many pixels (text wrapping changes → heights change).
const WIDTH_INVALIDATE_THRESHOLD: f32 = 50.0;

impl DesktopAI {
    pub(crate) fn render_chat_area(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let m = theme::palette(self.config.theme != "light");
        let font_size = self.config.font_size as f32;
        let total = self.current_conv.messages.len();
        let avail_w = ui.available_width();

        // ── Invalidate height cache on significant width change ──
        if (self.msg_heights_width - avail_w).abs() > WIDTH_INVALIDATE_THRESHOLD {
            self.msg_heights.clear();
            self.msg_heights_width = avail_w;
        }
        // Ensure the cache matches the message count
        if self.msg_heights.len() != total {
            self.msg_heights.resize(total, 0.0);
        }

        // ── Build cumulative height table (prefix sums) ──
        // cum[i] = y-offset of message i from the top of the scroll content.
        let mut cum: Vec<f32> = Vec::with_capacity(total + 1);
        cum.push(0.0);
        for i in 0..total {
            let h = if self.msg_heights[i] > 0.0 {
                self.msg_heights[i]
            } else {
                estimate_msg_height(
                    &self.current_conv.messages[i].content,
                    &self.current_conv.messages[i].role,
                    font_size,
                    avail_w,
                )
            };
            cum.push(cum[i] + h + MSG_SPACING);
        }
        let total_content_h = cum[total];

        ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false; 2])
            .show_viewport(ui, |ui, viewport| {
                let vis_top = viewport.min.y;
                let vis_bottom = viewport.max.y;

                // Binary search for the visible message range.
                let first_vis = cum
                    .partition_point(|&y| y < vis_top)
                    .saturating_sub(1)
                    .min(total.saturating_sub(1));
                let last_vis = cum
                    .partition_point(|&y| y <= vis_bottom)
                    .min(total);

                // Expand range with margin for smooth scrolling.
                let render_start = first_vis.saturating_sub(SCROLL_MARGIN);
                let render_end = (last_vis + SCROLL_MARGIN).min(total);

                // ── Reserve space above visible range ──
                if render_start > 0 {
                    let above_h = cum[render_start];
                    if above_h > 0.0 {
                        ui.allocate_space(vec2(ui.available_width(), above_h));
                    }
                }

                // ── Render only visible messages ──
                for i in render_start..render_end {
                    let msg = &self.current_conv.messages[i];
                    let is_user = msg.role == "user";
                    let y_before = ui.cursor().top();

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
                    ui.add_space(MSG_SPACING);

                    // ── Measure actual height and update cache ──
                    let actual_h = ui.cursor().top() - y_before;
                    if actual_h > 0.0 && (self.msg_heights[i] - actual_h).abs() > 1.0 {
                        self.msg_heights[i] = actual_h;
                    }
                }

                // ── Reserve space below visible range ──
                if render_end < total {
                    let below_h = total_content_h - cum[render_end];
                    if below_h > 0.0 {
                        ui.allocate_space(vec2(ui.available_width(), below_h));
                    }
                }

                // ── Streaming / generation indicator (always at bottom) ──
                if let Some(ref gen) = self.gen {
                    if gen.conv_id == self.current_conv.id {
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

                // ── 参考来源 ──
                if let Some((conv_id, sources)) = &self.last_sources {
                    if conv_id == &self.current_conv.id && !sources.is_empty() {
                        ui.add_space(8.0);
                        egui::CollapsingHeader::new(
                            RichText::new(format!("📚 参考来源（{} 条）", sources.len()))
                                .size(13.0)
                                .color(m.primary),
                        )
                        .id_salt("kb_sources")
                        .show(ui, |ui| {
                            for s in sources {
                                ui.label(
                                    RichText::new(format!("#{}  {}", s.rank, s.source))
                                        .size(12.5)
                                        .color(m.on_surface),
                                );
                                ui.label(
                                    RichText::new(&s.snippet)
                                        .size(11.5)
                                        .color(m.on_surface_variant),
                                );
                                ui.add_space(6.0);
                            }
                            ui.label(
                                RichText::new("按检索相关度排序；命中内容已作为参考注入回答")
                                    .size(10.5)
                                    .color(m.outline),
                            );
                        });
                    }
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
