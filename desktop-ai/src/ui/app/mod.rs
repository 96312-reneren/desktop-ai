//! DesktopAI application: module wiring and re-exports.
//!
//! The app is split into:
//!   state.rs    — DesktopAI struct, data types, constructor
//!   handlers.rs — business-logic methods (model load, generation, KB, ...)
//!   render.rs   — eframe::App impl (update loop, panel layout)
//!   chat.rs     — chat area + input bar rendering
//!   sidebar.rs  — sidebar rendering
//!   kb_panel.rs — knowledge-base + search panel rendering
//!   model_select.rs — model picker window
//!   settings.rs — settings window + confirm dialog
//!   theme.rs    — Material Design 3 palette and widget styles

mod chat;
mod handlers;
mod kb_panel;
mod model_select;
mod render;
mod settings;
mod sidebar;
pub(crate) mod state;
pub(crate) mod theme;

// Re-export only what sub-modules and external callers import via `super::`.
// Internal state types are accessed through `super::state::` paths directly.
pub(crate) use state::{apply_theme, ConfirmAction, DesktopAI};
