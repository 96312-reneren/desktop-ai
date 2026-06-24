#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use desktop_ai::app;

fn load_chinese_fonts() -> Option<Vec<u8>> {
    let font_paths = [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyhbd.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
        r"C:\Windows\Fonts\NotoSansCJKsc-VF.otf",
    ];

    for path in &font_paths {
        if let Ok(data) = std::fs::read(path) {
            log::info!("Loaded font: {}", path);
            return Some(data);
        }
    }

    log::warn!("No Chinese font found, CJK characters may not render");
    None
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Structured logging via tracing-subscriber with env-filter support.
    // Existing `log::info! / warn! / error!` macros are bridged via
    // `tracing-log` so no source changes are needed in other modules.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    // Ensure llama.dll is accessible
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();

    for path in &[
        std::path::PathBuf::from("llama.dll"),
        exe_dir.join("llama.dll"),
    ] {
        if path.exists() {
            std::env::set_current_dir(path.parent().unwrap_or(&exe_dir)).ok();
            break;
        }
    }

    // Pre-load Chinese font
    let chinese_font = load_chinese_fonts();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 680.0])
            .with_min_inner_size([750.0, 500.0])
            .with_title("桌面AI v5.8"),
        ..Default::default()
    };

    eframe::run_native(
        "桌面AI",
        options,
        Box::new(move |cc| {
            // Register Chinese font
            if let Some(ref font_data) = chinese_font {
                let mut fonts = egui::FontDefinitions::default();
                let font_data = egui::FontData::from_owned(font_data.clone());
                fonts.font_data.insert("chinese".into(), std::sync::Arc::new(font_data));
                // Make Chinese font the default proportional font only
                fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap()
                    .insert(0, "chinese".into());
                // Keep monospace as-is for code blocks
                cc.egui_ctx.set_fonts(fonts);
            }
            Ok(Box::new(app::DesktopAI::new()))
        }),
    )?;

    Ok(())
}
