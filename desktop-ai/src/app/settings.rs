// DesktopAI sub-module: settings window
use egui::{Color32, RichText, ScrollArea, TextEdit, vec2};
use crate::config;
use crate::model_catalog::find_model;
use super::{DesktopAI, ConfirmAction, apply_theme};

impl DesktopAI {
    pub(crate) fn render_settings(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
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

        ui.label(RichText::new("GPU 加速").strong());
        let mut gl = self.config.gpu_layers;
        ui.horizontal(|ui| {
            ui.label("层数:");
            if ui.add(egui::DragValue::new(&mut gl).range(0..=999).speed(1)).changed() {
                gl = gl.max(0);
            }
            if ui.button("自动").clicked() {
                gl = if self.gpu_info.is_empty() { 0 } else { 99 };
            }
        });
        if gl > 0 {
            ui.label(RichText::new(format!("将 {} 层模型卸载到 GPU (需重启模型)", gl))
                .size(10.0).color(Color32::from_rgb(100, 200, 255)));
            if !self.gpu_info.is_empty() {
                ui.label(RichText::new(format!("检测到 {} GPU, {:.1}GB VRAM", self.gpu_info[0].name, self.gpu_info[0].vram_gb))
                    .size(10.0).color(Color32::GRAY));
            }
        } else {
            ui.label(RichText::new("使用纯 CPU 推理 (0 = CPU)")
                .size(10.0).color(Color32::GRAY));
        }
        self.config.gpu_layers = gl;
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
        ui.checkbox(&mut self.config.kb_enabled, "启用本地知识库 (RAG)");
        ui.label(RichText::new("加载模型后可用。自动检索相关片段注入对话。").size(10.0).color(Color32::GRAY));
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
    }
}
