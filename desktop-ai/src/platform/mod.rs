//! 平台集成领域：Android 原生入口与桌面快捷方式。

#[cfg(target_os = "android")]
pub mod android;
pub(crate) mod shortcut;
