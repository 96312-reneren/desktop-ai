//! 界面领域：egui 应用、Markdown 渲染与桌面启动引导。
//!
//! 本模块只做子模块声明，不含具体实现：
//! - `app`：`eframe`/`egui` 应用本体，负责窗口、会话与交互，非 `Android` 平台编译；
//! - `markdown`：`Markdown` 到 `egui` 控件的渲染器，全平台可用；
//! - `startup`：桌面端启动引导（日志、字体、panic 钩子与 `eframe` 启动），非 `Android` 平台编译。

#[cfg(not(target_os = "android"))]
pub(crate) mod app;
pub(crate) mod markdown;
#[cfg(not(target_os = "android"))]
pub(crate) mod startup;
