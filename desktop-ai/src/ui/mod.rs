//! 界面领域：egui 应用、Markdown 渲染与桌面启动引导。

#[cfg(not(target_os = "android"))]
pub(crate) mod app;
pub(crate) mod markdown;
#[cfg(not(target_os = "android"))]
pub(crate) mod startup;
