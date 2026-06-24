// DesktopAI sub-module: sidebar
use super::DesktopAI;
use crate::conversation::Conversation;
use egui::{vec2, Color32, RichText, ScrollArea, TextEdit};

impl DesktopAI {
    pub(crate) fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.heading("桌面AI");
        ui.label(RichText::new("v5.8").size(10.0).color(Color32::GRAY));
        ui.add_space(8.0);

        if ui.button("+ 新对话").clicked() {
            self.new_conversation();
        }

        ui.add_space(4.0);
        ui.separator();
        ui.label(RichText::new("对话历史").size(11.0).color(Color32::GRAY));
        ui.add_sized(
            vec2(ui.available_width(), 20.0),
            TextEdit::singleline(&mut self.conv_filter).hint_text("搜索对话... Ctrl+F"),
        );
        ui.add_space(2.0);

        ScrollArea::vertical().max_height(230.0).show(ui, |ui| {
            let convs = Conversation::list_all();
            let filter = self.conv_filter.trim().to_lowercase();
            for conv in &convs {
                if !filter.is_empty()
                    && !conv.title.to_lowercase().contains(&filter)
                    && !conv.id.contains(&filter)
                {
                    continue;
                }
                ui.horizontal(|ui| {
                    let title = if conv.title.len() > 18 {
                        format!("{}...", &conv.title[..18])
                    } else {
                        conv.title.clone()
                    };
                    let active = conv.id == self.current_conv.id;
                    if ui.selectable_label(active, &title).clicked() {
                        self.load_conversation(&conv.id);
                    }
                    if ui.button("✕").clicked() {
                        self.delete_conversation(&conv.id);
                    }
                });
                ui.label(
                    RichText::new(format!("{} 条消息", conv.message_count))
                        .size(10.0)
                        .color(Color32::GRAY),
                );
            }
        });

        ui.add_space(8.0);
        ui.separator();
        if ui.button("切换模型").clicked() {
            self.show_model_select = true;
        }
        if ui.button("搜索").clicked() {
            self.show_search_panel = !self.show_search_panel;
        }
        if ui.button("知识库").clicked() {
            self.show_kb_panel = !self.show_kb_panel;
        }
        ui.add_space(4.0);
        ui.separator();
        ui.label(RichText::new("对话").size(11.0).color(Color32::GRAY));
        if ui.button("导出当前对话").clicked() {
            self.export_current_conversation();
        }
        if ui.button("导入对话").clicked() {
            self.import_conversation();
        }
    }
}
