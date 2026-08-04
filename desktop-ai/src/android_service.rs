//! Android service entry — native side of the hybrid app.
//!
//! The Java `MainActivity` calls `startRust` (via JNI) with the app's
//! internal and external data dirs; this loads the first available model and
//! starts the OpenAI-compatible API server bound to 127.0.0.1. The UI is a
//! WebView chat page that talks to the API server through a JS bridge (see
//! `android/app/src/main/kotlin/.../MainActivity.kt`). This route replaces the
//! abandoned egui/winit Android port, which suffered from unfixed IME focus
//! bugs in the winit Android backend.

use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;
use std::sync::{Arc, Mutex};

use crate::config;
use crate::inference::LlamaInference;

#[link(name = "log")]
extern "C" {
    fn __android_log_print(
        prio: i32,
        tag: *const std::os::raw::c_char,
        fmt: *const std::os::raw::c_char,
        ...
    ) -> i32;
}

/// Log to logcat (tag `DesktopAI-N`); used on Android where stderr is not
/// routed to logcat for regular Activities.
pub(crate) fn android_log(msg: &str) {
    let tag = b"DesktopAI-N\0";
    let m = msg.replace('%', "%%");
    let Ok(m) = std::ffi::CString::new(m) else { return };
    unsafe {
        __android_log_print(
            3,
            tag.as_ptr() as *const _,
            b"%s\0".as_ptr() as *const _,
            m.as_ptr(),
        );
    }
}

/// Placeholder required by android-activity's native-activity runtime: the
/// .so must export `android_main` or `System.loadLibrary` fails to resolve
/// it. The real entry is `startRust` via JNI; a plain Activity never calls
/// this.
#[no_mangle]
pub extern "C" fn android_main(_app: android_activity::AndroidApp) {
    std::process::abort();
}

/// Called by Java: `startRust(internalDir, externalDir, apiPort)`.
#[no_mangle]
pub extern "C" fn Java_com_desktopai_android_MainActivity_startRust(
    mut env: JNIEnv,
    _class: JClass,
    internal_dir: JString,
    external_dir: JString,
    api_port: jni::sys::jint,
) {
    let internal: String = env
        .get_string(&internal_dir)
        .map(|s| s.into())
        .unwrap_or_default();
    let external: String = env
        .get_string(&external_dir)
        .map(|s| s.into())
        .unwrap_or_default();
    android_log(&format!("startRust: internal={} external={} port={}", internal, external, api_port));

    // Logs and panics go to the internal dir (always writable). The external
    // dir is preferred for models (adb-pushable) but is not guaranteed
    // writable on every launch.
    let log_dir = if !internal.is_empty() {
        std::path::Path::new(&internal).join("DesktopAI")
    } else {
        std::path::Path::new("/data/data/com.desktopai.android/files/DesktopAI").to_path_buf()
    };
    let _ = std::fs::create_dir_all(&log_dir);
    let panic_file = log_dir.join("panic.txt");

    std::panic::set_hook(Box::new(move |info| {
        let msg = format!("PANIC: {}\n", info);
        let _ = std::fs::write(&panic_file, &msg);
        android_log(&msg);
    }));

    if !internal.is_empty() {
        std::env::set_var("HOME", &internal);
    }
    if !external.is_empty() {
        std::env::set_var("ANDROID_EXTERNAL_DIR", &external);
    }

    // tracing to a file; stderr is not routed to logcat for regular
    // Activities, but android_log above covers the critical paths.
    let appender =
        std::panic::catch_unwind(|| tracing_appender::rolling::never(&log_dir, "rust.log"))
            .unwrap_or_else(|_| tracing_appender::rolling::never("/data/local/tmp", "rust.log"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_ansi(false)
        .with_writer(appender)
        .try_init();

    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            start_service(api_port as u16);
        }));
        if let Err(e) = result {
            let msg = format!("SERVICE PANIC: {:?}\n", e);
            android_log(&msg);
            let pf = if !external.is_empty() {
                format!("{}/DesktopAI/panic.txt", external)
            } else {
                "panic.txt".to_string()
            };
            let _ = std::fs::write(&pf, msg);
        }
    });
}

fn start_service(api_port: u16) {
    // 1. Pick the first downloaded model.
    let models_dir = config::models_dir();
    let mut model_path = None;
    if let Ok(entries) = std::fs::read_dir(&models_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "gguf") {
                model_path = Some(path);
                break;
            }
        }
    }
    let Some(model_path) = model_path else {
        log::error!("no model found in {}", models_dir.display());
        android_log(&format!("start_service: NO MODEL in {}", models_dir.display()));
        return;
    };
    log::info!("loading model: {}", model_path.display());

    // 2. Load the model (background, may take a while).
    let n_ctx = 2048u32;
    let n_threads = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4);
    let inf = match LlamaInference::load_ex(&model_path.to_string_lossy(), n_ctx, n_threads, 0) {
        Ok(i) => i,
        Err(e) => {
            log::error!("model load failed: {}", e);
            android_log(&format!("start_service: MODEL LOAD FAILED {:?}", e));
            return;
        }
    };
    let inf = Arc::new(Mutex::new(inf));
    log::info!("model loaded");
    android_log("start_service: model loaded");

    // 3. Start the OpenAI-compatible API server. The token is fixed on
    // Android so the WebView UI can talk to it.
    let mut cfg = config::Config::default();
    cfg.api_token = "desktopai".to_string();
    let name = "desktop-ai".to_string();
    // Keep the server alive: dropping it sets the stop flag and the
    // listener thread exits immediately.
    let mut _server = crate::api_server::ApiServer::start(inf, api_port, name, cfg.api_token.clone());
    log::info!("api server on 127.0.0.1:{}", api_port);
    android_log(&format!("start_service: api server on 127.0.0.1:{}", api_port));

    // Keep the server alive (dropping ApiServer stops the listener) and
    // keep the process alive.
    loop {
        let _ = &mut _server;
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
