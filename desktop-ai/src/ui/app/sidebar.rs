// DesktopAI sub-module: sidebar (Material Design 3 styling)
use super::{theme, DesktopAI};
use egui::{vec2, RichText, ScrollArea, TextEdit};

impl DesktopAI {
    pub(crate) fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        let m = theme::palette(self.config.theme != "light");

        ui.label(RichText::new("桌面AI").size(20.0).color(m.primary));
        ui.label(
            RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                .size(10.0)
                .color(m.on_surface_variant),
        );
        ui.add_space(8.0);

        // M3 filled tonal "new chat" button.
        if ui
            .add_sized(
                vec2(ui.available_width(), 36.0),
                theme::tonal_button(RichText::new("+  新对话").size(14.0), m),
            )
            .clicked()
        {
            self.new_conversation();
        }

        ui.add_space(6.0);

        // ── Current model indicator ──
        match &self.loaded_model_name {
            Some(name) => {
                ui.label(
                    RichText::new(format!("● {}", name))
                        .size(11.0)
                        .color(m.success),
                );
            }
            None => {
                ui.label(RichText::new("● 未加载模型").size(11.0).color(m.error));
            }
        }
        if ui.small_button("选择 / 重新加载模型").clicked() {
            self.show_model_select = true;
        }

        ui.add_space(4.0);
        ui.separator();
        ui.label(
            RichText::new("对话历史")
                .size(11.0)
                .color(m.on_surface_variant),
        );
        ui.add_sized(
            vec2(ui.available_width(), 24.0),
            TextEdit::singleline(&mut self.conv_filter).hint_text("搜索对话... Ctrl+F"),
        );
        ui.add_space(2.0);

        // Refresh the cached conversation list only when something changed.
        self.refresh_conv_cache();
        // Clone the cache so mutation (load/delete) inside the loop is fine.
        let convs = self.conv_cache.clone();
        ScrollArea::vertical().max_height(230.0).show(ui, |ui| {
            let filter = self.conv_filter.trim().to_lowercase();
            for conv in &convs {
                if !filter.is_empty()
                    && !conv.title.to_lowercase().contains(&filter)
                    && !conv.id.contains(&filter)
                {
                    continue;
                }
                ui.horizontal(|ui| {
                    // Char-boundary safe truncation (was a byte slice: a
                    // Chinese title could panic).
                    let title: String = conv.title.chars().take(18).collect();
                    let title = if conv.title.chars().count() > 18 {
                        format!("{}...", title)
                    } else {
                        title
                    };
                    let active = conv.id == self.current_conv.id;
                    if ui.selectable_label(active, &title).clicked() {
                        let id = conv.id.clone();
                        self.load_conversation(&id);
                    }
                    if ui.button("✕").clicked() {
                        let id = conv.id.clone();
                        self.delete_conversation(&id);
                    }
                });
                ui.label(
                    RichText::new(format!("{} 条消息", conv.message_count))
                        .size(10.0)
                        .color(m.on_surface_variant),
                );
            }
        });

        ui.add_space(8.0);
        ui.separator();
        if ui.button("搜索").clicked() {
            self.show_search_panel = !self.show_search_panel;
        }
        if ui.button("知识库").clicked() {
            self.show_kb_panel = !self.show_kb_panel;
        }
        ui.add_space(4.0);
        ui.separator();
        ui.label(RichText::new("对话").size(11.0).color(m.on_surface_variant));
        if ui.button("导出当前对话").clicked() {
            self.export_current_conversation();
        }
        if ui.button("导入对话").clicked() {
            self.import_conversation();
        }
    }
}
