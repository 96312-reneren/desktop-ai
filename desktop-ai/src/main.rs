//! 桌面端二进制入口壳。
//!
//! 本文件不承载任何应用逻辑：桌面目标调用 `desktop_ai::run()`（即 `ui::startup`
//! 的引导逻辑，负责日志、字体、panic 钩子与 eframe 启动），Android 目标改用
//! cdylib + JNI 入口（见 `platform::android`），这里的 `main` 为空实现，仅为满足
//! bin target 而存在。应用本体全部在 `lib.rs`，使入口保持最小、易审计。
//!
//! 顶部 `windows_subsystem = "windows"` 仅在非 debug 构建生效，用于让 Windows
//! 以 GUI 子系统启动、避免弹出控制台窗口。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Android 构建使用 cdylib 入口（见 `platform::android::android_main`），
/// 该 `main` 仅为满足 crate 的 bin target 而存在。
#[cfg(target_os = "android")]
fn main() {}

/// 桌面平台主入口：转发到 `desktop_ai::run()` 启动应用，并把其错误向上传播；
/// 返回 `Err` 时由运行时打印错误并以非零状态退出。真正的装配逻辑在 `ui::startup`。
#[cfg(not(target_os = "android"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    desktop_ai::run()
}
