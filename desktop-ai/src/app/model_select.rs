// DesktopAI sub-module: model select window
use super::{get_total_ram_gb, DesktopAI};
use crate::config;
use egui::{Color32, RichText};

impl DesktopAI {
    pub(crate) fn render_model_select(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("选择一个模型（可同时下载多个）")
                .size(14.0)
                .strong(),
        );
        ui.add_space(4.0);

        // Hardware info
        ui.label(
            RichText::new(format!("你的设备: {} 核 CPU", self.cpu_cores))
                .size(11.0)
                .color(Color32::GRAY),
        );
        for gpu in &self.gpu_info {
            ui.label(
                RichText::new(format!("显卡: {} ({:.1} GB VRAM)", gpu.name, gpu.vram_gb))
                    .size(11.0)
                    .color(Color32::GRAY),
            );
        }
        if let Some(ref warn) = self.ram_warning {
            ui.label(
                RichText::new(warn)
                    .size(11.0)
                    .color(Color32::from_rgb(255, 200, 50)),
            );
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
                        ui.label(
                            RichText::new(tag)
                                .size(10.0)
                                .background_color(Color32::from_rgb(31, 106, 165))
                                .color(Color32::WHITE),
                        );
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
                            ui.label(
                                RichText::new("✓ 已下载").color(Color32::from_rgb(76, 175, 80)),
                            );
                            if ui.button("使用").clicked() {
                                self.config.selected_model_id = Some(model.id.clone());
                                config::save_config(&self.config);
                                self.load_selected_model();
                                self.show_model_select = false;
                            }
                        } else if ui.button("下载").clicked() {
                            self.start_download(&model.id);
                        }
                    });
                });
                ui.label(RichText::new(&model.desc).size(11.0).color(Color32::GRAY));
                ui.label(
                    RichText::new(format!("约 {:.2} GB", model.size_gb))
                        .size(11.0)
                        .color(Color32::from_rgb(76, 175, 80)),
                );

                let ram_gb = get_total_ram_gb();
                let rec_ram = model.size_gb * 3.0 + 1.0;
                if ram_gb > 0.0 && ram_gb < rec_ram {
                    ui.label(
                        RichText::new(format!("⚠ 推荐 {:.0} GB 内存，你的设备可能不足", rec_ram))
                            .size(10.0)
                            .color(Color32::from_rgb(255, 165, 0)),
                    );
                }
            });
        }
        if ui.button("关闭").clicked() {
            self.show_model_select = false;
        }
    }
}
