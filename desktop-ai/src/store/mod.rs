//! 本地持久化领域：配置、SQLite 连接与对话历史。
//!
//! 子模块：
//! - `config`：数据根目录发现、`Config`/`ModelInfo` 结构及其 JSON 读写。
//! - `db`：`SQLite` 连接封装（`WAL`、外键、`busy_timeout`）。
//! - `conversation`：基于 `SQLite` 的对话历史存取。
//!
//! 上层 UI 与 `llm` 通过 `config` 的路径辅助函数定位数据根目录；根目录可被
//! 环境变量或便携模式覆盖（见 `config::data_root`）。

pub(crate) mod config;
pub(crate) mod conversation;
pub(crate) mod db;
