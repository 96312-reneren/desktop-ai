#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Android 构建使用 cdylib 入口（见 `platform::android::android_main`），
/// 该 `main` 仅为满足 crate 的 bin target 而存在。
#[cfg(target_os = "android")]
fn main() {}

#[cfg(not(target_os = "android"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    desktop_ai::run()
}
