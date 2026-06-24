// DesktopAI sub-module: knowledge-base panel + search panel
use super::DesktopAI;
use egui::{vec2, Color32, RichText, ScrollArea, TextEdit};

impl DesktopAI {
    pub(crate) fn render_search_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_sized(
                vec2(ui.available_width() - 60.0, 24.0),
                TextEdit::singleline(&mut self.search_query).hint_text("输入搜索关键词..."),
            );
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
            ui.label(
                RichText::new(format!("{} 条结果", self.search_results.len()))
                    .size(11.0)
                    .color(Color32::GRAY),
            );
            ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                for result in &self.search_results {
                    ui.group(|ui| {
                        ui.label(RichText::new(&result.title).size(12.0).strong());
                        if !result.url.is_empty() {
                            ui.label(
                                RichText::new(&result.url)
                                    .size(10.0)
                                    .color(Color32::from_rgb(100, 180, 255)),
                            );
                        }
                        if !result.snippet.is_empty() {
                            ui.label(RichText::new(&result.snippet).size(11.0));
                        }
                    });
                    ui.add_space(3.0);
                }
            });
        }
    }

    pub(crate) fn render_kb_panel(&mut self, ui: &mut egui::Ui) {
        // Close button at top
        ui.horizontal(|ui| {
            ui.heading(RichText::new("知识库").size(14.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(
                        RichText::new("×")
                            .size(16.0)
                            .color(Color32::from_rgb(200, 80, 80)),
                    )
                    .clicked()
                {
                    self.show_kb_panel = false;
                }
            });
        });
        ui.separator();
        ui.add_space(4.0);

        ScrollArea::vertical().max_height(500.0).show(ui, |ui| {
        ui.label(RichText::new("添加文档").size(13.0).strong());
        ui.add_space(4.0);

        // File input via path
        ui.label(RichText::new("从文件载入 (txt/md):").size(11.0).color(Color32::GRAY));
        ui.add_sized(vec2(ui.available_width(), 18.0),
            TextEdit::singleline(&mut self.kb_title).hint_text("文件路径，如: C:\\docs\\readme.txt"));
        if ui.add_sized(vec2(ui.available_width(), 24.0),
            egui::Button::new("载入文件 (分块+向量化)")
        ).clicked() {
            self.pick_and_index_file();
        }
        ui.add_space(4.0);

        // Web crawl
        ui.label(RichText::new("从网页爬取:").size(11.0).color(Color32::GRAY));
        ui.horizontal(|ui| {
            ui.add_sized(vec2(ui.available_width() - 100.0, 20.0),
                TextEdit::singleline(&mut self.kb_url).hint_text("https://..."));
            ui.label("层数:");
            if ui.add(egui::DragValue::new(&mut self.kb_crawl_depth)
                .range(1..=3).speed(0)
            ).changed() {
                self.kb_crawl_depth = self.kb_crawl_depth.clamp(1, 3);
            }
        });
        ui.horizontal(|ui| {
            if ui.add_sized(vec2(ui.available_width() - 12.0, 22.0),
                egui::Button::new("爬取")
            ).clicked() {
                self.crawl_url_to_kb();
            }
        });
        ui.add_space(4.0);

        // Indexing progress
        if self.kb_indexing {
            ui.add(egui::ProgressBar::new(self.kb_index_progress)
                .desired_width(ui.available_width())
                .text(&self.kb_index_status));
            ui.add_space(2.0);
        }

        // Manual paste
        ui.label(RichText::new("或粘贴文本:").size(11.0).color(Color32::GRAY));
        ui.add_sized(vec2(ui.available_width(), 18.0),
            TextEdit::singleline(&mut self.kb_title).hint_text("文档标题或文件路径"));
        ui.add_sized(vec2(ui.available_width(), 60.0),
            TextEdit::multiline(&mut self.kb_content).hint_text("粘贴内容..."));
        if ui.add_sized(vec2(ui.available_width(), 26.0),
            egui::Button::new("添加文本 (分块+向量化)")
        ).clicked() {
            if self.kb_content.trim().is_empty() {
                self.pick_and_index_file();
            } else {
                self.paste_and_index_text();
            }
        }
        ui.add_space(8.0);

        ui.separator();
        ui.label(RichText::new("已索引文档").size(13.0).strong());
        let docs = self.vector_store.documents().to_vec();
        if docs.is_empty() {
            ui.label(RichText::new("暂无文档。通过上方按钮选择文件或粘贴文本。")
                .size(11.0).color(Color32::GRAY));
        } else {
            ui.label(RichText::new(format!("共 {} 个文档", docs.len()))
                .size(11.0).color(Color32::GRAY));
            ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                for doc in &docs {
                    let total_chars: usize = doc.chunks.iter().map(|c| c.text.len()).sum();
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            let title = if doc.title.len() > 30 {
                                format!("{}...", &doc.title[..30])
                            } else { doc.title.clone() };
                            ui.label(RichText::new(&title).size(12.0).strong());
                            if ui.button("删除").clicked() {
                                let id = doc.id.clone();
                                self.delete_kb_document(&id);
                            }
                        });
                        ui.label(RichText::new(
                            format!("{} 分块, {} 字符 | {}",
                                doc.chunks.len(), total_chars, &doc.created_at[..10]))
                            .size(10.0).color(Color32::GRAY));
                    });
                    ui.add_space(2.0);
                }
            });
        }

        ui.add_space(4.0);
        ui.separator();
        ui.label(RichText::new("AI 工作区 (沙盒)").size(13.0).strong());
        ui.label(RichText::new(
            format!("路径: {}", self.sandbox.root_path().display())
        ).size(9.0).color(Color32::GRAY));

        if let Ok(entries) = self.sandbox.list("") {
            if entries.is_empty() {
                ui.label(RichText::new("工作区为空。AI 生成的回答可保存到此。")
                    .size(11.0).color(Color32::GRAY));
            } else {
                ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                    for entry in &entries {
                        ui.horizontal(|ui| {
                            let icon = if entry.is_dir { "[D]" } else { "[F]" };
                            let color = if entry.is_dir {
                                Color32::from_rgb(100, 180, 255)
                            } else {
                                Color32::from_rgb(200, 200, 200)
                            };
                            let preview = entry.name.clone();
                            ui.label(RichText::new(format!("{}{}", icon, preview))
                                .size(11.0).color(color));
                        });
                    }
                });
            }
        }

        ui.add_space(4.0);
        ui.label(RichText::new(
            "提示: 支持文件(txt/md/pdf/html)、粘贴文本、网页爬取。所有输入均经清洗管道转为纯文本后索引。")
            .size(10.0).color(Color32::GRAY));
        }); // ScrollArea
    }
}
