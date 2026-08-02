#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use desktop_ai::app;

fn font_candidates() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &[
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\msyhbd.ttc",
            r"C:\Windows\Fonts\simhei.ttf",
            r"C:\Windows\Fonts\simsun.ttc",
            r"C:\Windows\Fonts\NotoSansCJKsc-VF.otf",
        ]
    }
    #[cfg(target_os = "linux")]
    {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        ]
    }
    #[cfg(target_os = "macos")]
    {
        &[
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/Songti.ttc",
        ]
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        &[]
    }
}

fn load_chinese_fonts() -> Option<Vec<u8>> {
    for path in font_candidates() {
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
    // Logs go to stderr AND a daily-rotating file in the data dir so
    // crashes can be diagnosed after the fact.
    use tracing_subscriber::fmt::writer::MakeWriterExt;

    let log_dir = desktop_ai::config::log_dir();
    let file_appender = tracing_appender::rolling::daily(&log_dir, "desktop-ai.log");
    let (file_writer, log_guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_writer(std::io::stderr.and(file_writer))
        .init();
    let _log_guard = log_guard; // keep the non-blocking writer alive

    // Ensure the llama shared library is accessible
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();
    let lib_name = desktop_ai::ffi::llama_library_name();

    for path in &[std::path::PathBuf::from(lib_name), exe_dir.join(lib_name)] {
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
            .with_title("桌面AI v6.1"),
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
                fonts
                    .font_data
                    .insert("chinese".into(), std::sync::Arc::new(font_data));
                // Make Chinese font the default proportional font only
                fonts
                    .families
                    .get_mut(&egui::FontFamily::Proportional)
                    .unwrap()
                    .insert(0, "chinese".into());
                // Keep monospace as-is for code blocks
                cc.egui_ctx.set_fonts(fonts);
            }
            Ok(Box::new(app::DesktopAI::new()))
        }),
    )?;

    Ok(())
}
