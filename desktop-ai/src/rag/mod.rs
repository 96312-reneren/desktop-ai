//! RAG 领域：文本分块 / 清洗、网页爬取与搜索、本地向量库。
//!
//! 本模块是知识库（KB）子系统的领域层：`cleaner` 把抓取到的原始文本
//! （`HTML`、markdown、纯文本）清洗为可入库的纯文本，`chunker` 再把长文按句
//! 边界切成带重叠的定长片段，随后由 `crawler`、`vector_store` 等完成抓取、
//! 嵌入与检索。其中 `chunk_text`、`clean_text` 经 crate 根 `lib.rs` 重新导出
//! 为对外 API。

pub(crate) mod chunker;
pub(crate) mod cleaner;
pub(crate) mod crawler;
pub(crate) mod kb_job;
pub(crate) mod robots;
pub(crate) mod search;
pub(crate) mod vector_store;
