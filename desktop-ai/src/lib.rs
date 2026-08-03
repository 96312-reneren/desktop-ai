pub mod api_server;
pub mod app;
pub mod chunker;
pub mod cleaner;
pub mod config;
pub mod conversation;
pub mod crawler;
pub mod db;
pub mod downloader;
pub mod embedding;
pub mod ffi;
pub mod inference;
pub mod markdown;
pub mod model_catalog;
pub mod sandbox;
pub mod search;
pub mod shortcut;
pub mod vector_store;

/// Android entry point (loaded by the native-activity runtime as a cdylib).
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn android_main(app: android_activity::AndroidApp) {
    use eframe::egui;

    // Point HOME at the app-private files dir so data_root() resolves to a
    // writable location (the native-activity runtime doesn't set HOME).
    if let Some(dir) = app.internal_data_path() {
        std::env::set_var("HOME", &dir);
        log::info!("internal storage: {}", dir.display());
    }

    // Route logs to stderr (adb logcat shows them).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();

    let options = eframe::NativeOptions {
        android_app: Some(app),
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 680.0])
            .with_min_inner_size([500.0, 400.0]),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "桌面AI",
        options,
        Box::new(move |cc| {
            // CJK font from the Android system font directory.
            for path in [
                "/system/fonts/NotoSansCJK-Regular.ttc",
                "/system/fonts/NotoSansCJKsc-Regular.otf",
                "/system/fonts/DroidSansFallback.ttf",
            ] {
                if let Ok(data) = std::fs::read(path) {
                    let mut fonts = egui::FontDefinitions::default();
                    fonts
                        .font_data
                        .insert("chinese".into(), std::sync::Arc::new(egui::FontData::from_owned(data)));
                    fonts
                        .families
                        .get_mut(&egui::FontFamily::Proportional)
                        .unwrap()
                        .insert(0, "chinese".into());
                    cc.egui_ctx.set_fonts(fonts);
                    log::info!("Loaded font: {}", path);
                    break;
                }
            }
            Ok(Box::new(app::DesktopAI::new()))
        }),
    );
}
