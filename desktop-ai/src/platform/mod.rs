//! 平台集成领域：Android 原生入口与桌面快捷方式。
//!
//! 汇集与操作系统强相关、且不属于渲染/UI 层的代码：
//! - `android`：仅 Android 目标编译的 JNI/NDK 桥，负责加载模型并启动本地 API server；
//! - `hardware`：CPU 核数、内存与 GPU 探测，供模型选择器评估机器能否承载目标模型；
//! - `shortcut`：首次运行时在 Windows/Linux 桌面及应用菜单创建启动项。

#[cfg(target_os = "android")]
pub mod android;
pub(crate) mod hardware;
pub(crate) mod shortcut;
