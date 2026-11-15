//! 知识库的 SQLite 持久化层：`documents`/`chunks` 双表存储文档与分块，
//! `chunks_fts` 是手工双写的独立 FTS5 索引（非 content-linked）。
//!
//! 对外：向上服务 RAG 检索流程，向下依赖 [`crate::store::db`] 取连接、
//! 依赖 `EmbeddingEngine` 生成向量。两条检索路径并存：向量余弦相似度
//! （`search_by_vector`，embedding 以 BLOB 存储）与 FTS5 关键词检索
//! （`search_text`）。设计边界：分块行随文档 `ON DELETE CASCADE`，但 FTS
//! 行不受级联影响，故由 [`rebuild_fts_if_stale`] 做孤儿感知的自愈重建。
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::llm::embedding::EmbeddingEngine;
use crate::store::db::Db;

/// 一个持久化的文本分块及其 embedding 向量。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredChunk {
    pub text: String,
    pub embedding: Vec<f32>,
}

/// 入库文档的来源元数据：来源类型 + 可选的定位信息（URL / 文件路径）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct DocMetadata {
    /// 来源类型："paste" | "file" | "crawl"；旧库迁移后为空串（未知）。
    pub source_type: String,
    /// 抓取来源的 URL 或文件入库的原始路径；粘贴文本为 None。
    pub source_url: Option<String>,
}

/// 来源类型的展示徽标：粘贴📋 / 文件📄 / 抓取🌐 / 未知（空串）␣
pub(crate) fn source_type_badge(source_type: &str) -> &'static str {
    match source_type {
        "paste" => "📋 ",
        "file" => "📄 ",
        "crawl" => "🌐 ",
        _ => "",
    }
}

/// 一次检索命中的分块：正文、可选的相似度分数与来源标题。
#[derive(Debug, Clone)]
pub(crate) struct SearchHit {
    pub chunk: String,
    /// Retrieval score when the backend produces one (cosine similarity,
    /// 0..1); `None` for FTS5 keyword hits, which have no similarity
    /// semantics. Never rendered as a percentage — it is a ranking signal,
    /// not a probability, and cosine scores can even be negative.
    #[allow(dead_code)]
    pub score: Option<f32>,
    pub source: String,
    pub metadata: DocMetadata,
}

/// A retrieval reference surfaced to the user under an answer ("参考来源"):
/// rank (not a similarity percentage — cosine scores are not a probability
/// and can even be negative), the document title / URL, and a short excerpt.
#[derive(Debug, Clone)]
pub(crate) struct SourceRef {
    /// 1-based rank within this retrieval.
    pub rank: usize,
    pub source: String,
    pub snippet: String,
    /// 来源元数据；供「参考来源」区渲染类型徽标。
    pub metadata: DocMetadata,
}

impl SourceRef {
    /// 由检索命中构造用户可见的来源引用；`rank` 按命中顺序自 1 递增，
    /// `snippet` 取分块前 80 个字符。
    pub(crate) fn from_hits(hits: &[SearchHit]) -> Vec<Self> {
        hits.iter()
            .enumerate()
            .map(|(i, hit)| Self {
                rank: i + 1,
                source: hit.source.clone(),
                snippet: hit.chunk.chars().take(80).collect(),
                metadata: hit.metadata.clone(),
            })
            .collect()
    }
}

/// 一篇完整持久化的文档：元信息加其全部分块（含 embedding）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredDocument {
    pub id: String,
    pub title: String,
    pub chunks: Vec<StoredChunk>,
    pub created_at: String,
    pub metadata: DocMetadata,
}

/// SQLite-backed vector store.
///
/// Concurrency: embedding inference happens *before* taking the DB lock;
/// inside the lock only short SQL transactions run (see [`crate::store::db`]).
///
/// The embedding engine is guarded by a `Mutex`: `EmbeddingEngine` is
/// `Send` but not `Sync` (it owns raw FFI pointers), and indexing runs on a
/// background thread while the UI thread may call `embed_query` at the same
/// time. Both paths lock the same mutex, so inference is serialised and the
/// engine is only ever touched by one thread at a time. Keep heavy work
/// inside the lock to a minimum.
pub(crate) struct VectorStore {
    /// SQLite 连接封装（负责运行时 PRAGMA 与事务）。
    db: Db,
    /// 嵌入后端；[`VectorStore::set_engine`] 之前为 `None`。
    engine: Option<Arc<Mutex<EmbeddingEngine>>>,
}

/// 当前 schema 版本；用作 `PRAGMA user_version` 门控一次性迁移：
/// v1 = JSON 导入，v2 = documents 增加来源元数据列。
const SCHEMA_VERSION: i64 = 2;

/// 建表 DDL：`documents`/`chunks` 双表（分块外键 `ON DELETE CASCADE`），
/// 外加手工双写的独立 FTS5 表 `chunks_fts`（非 content-linked）。
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS documents (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    source_type TEXT NOT NULL DEFAULT '',
    source_url  TEXT
);
CREATE TABLE IF NOT EXISTS chunks (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    doc_id    TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    idx       INTEGER NOT NULL,
    text      TEXT NOT NULL,
    embedding BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_chunks_doc ON chunks(doc_id);
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(doc_id UNINDEXED, text);
";

/// 将 f32 向量按小端序序列化为 BLOB（与 [`blob_to_embed`] 互逆）。
fn embed_to_blob(v: &[f32]) -> Vec<u8> {
    let mut bytes = vec![0u8; v.len() * 4];
    for (i, x) in v.iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&x.to_le_bytes());
    }
    bytes
}

/// 将小端序 BLOB 反序列化为 f32 向量。
///
/// 前提：`bytes` 长度为 4 的倍数（由 [`embed_to_blob`] 保证）；
/// 内部 `copy_nonoverlapping` 依赖该前提，否则会越界。
fn blob_to_embed(bytes: &[u8]) -> Vec<f32> {
    let mut out = vec![0f32; bytes.len() / 4];
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.as_mut_ptr() as *mut u8, bytes.len());
    }
    out
}

impl VectorStore {
    /// 打开（或新建）知识库并初始化 schema。
    ///
    /// `store_dir` 下的 `kb.db` 打开失败时降级到进程级临时库，保证功能可用而非崩溃。
    /// 初始化依次执行：建表、按 `user_version` 门控的分段迁移（v0→v1 JSON 导入、
    /// v1→v2 补来源元数据列）、[`rebuild_fts_if_stale`] 的 FTS 自愈重建；
    /// 这些步骤出错只记日志、不阻断构造。
    pub(crate) fn new(store_dir: &std::path::Path) -> Self {
        let db = crate::store::db::open(&store_dir.join("kb.db")).unwrap_or_else(|e| {
            log::error!("failed to open kb.db: {} — using in-memory fallback", e);
            crate::store::db::open(
                &std::env::temp_dir()
                    .join(format!("desktop_ai_kb_fallback_{}.db", std::process::id())),
            )
            .expect("fallback kb db")
        });
        if let Err(e) = db.with_conn(|c| {
            c.execute_batch(SCHEMA)?;
            let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            // v0 → v1：导入遗留 vector_store.json（保持原有行为）。
            if version < 1 {
                if let Err(e) = migrate_from_json(store_dir, c) {
                    log::error!("kb JSON migration failed: {}", e);
                }
            }
            // v1 → v2：documents 增加来源元数据列。用 PRAGMA table_info 检查列
            // 是否存在，保证上次迁移中途失败后重跑也不会因重复 ALTER 报错。
            if version < 2 {
                add_source_columns_if_missing(c)?;
            }
            if version < SCHEMA_VERSION {
                c.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            }
            // One-time FTS backfill: index any chunks that were stored before
            // the FTS table existed (or if a partial index was interrupted).
            rebuild_fts_if_stale(c)?;
            Ok(())
        }) {
            log::error!("kb schema init failed: {}", e);
        }
        Self { db, engine: None }
    }

    /// Install an embedding backend.
    ///
    /// Takes **ownership** of `engine`. After this call the caller MUST NOT
    /// retain any reference to the `EmbeddingEngine` — it owns raw FFI
    /// pointers (`*mut ffi::LlamaModel`, `*mut ffi::LlamaContext`) and is
    /// NOT `Sync`. All subsequent embedding access goes through
    /// `VectorStore::embed_query` / `add_document` / `search` which borrow
    /// `&self` internally.
    pub(crate) fn set_engine(&mut self, engine: EmbeddingEngine) {
        self.engine = Some(Arc::new(Mutex::new(engine)));
    }

    /// 是否已装入嵌入后端；未装时向量检索路径不可用。
    pub(crate) fn has_engine(&self) -> bool {
        self.engine.is_some()
    }

    /// 加载全部文档及其分块（含向量）；读取失败时返回空列表而非报错。
    pub(crate) fn documents(&self) -> Vec<StoredDocument> {
        self.db
            .with_conn(|c| load_all_documents(c))
            .unwrap_or_default()
    }

    /// 切分文本、逐块求 embedding，并作为一篇新文档持久化。
    ///
    /// 契约：须先 [`VectorStore::set_engine`]，否则返回错误；文本切不出任何块时同样报错。
    /// 并发：embedding 推断在加锁外完成，仅写库的短事务持锁（见结构体文档）。
    pub(crate) fn add_document(
        &self,
        title: &str,
        text: &str,
        chunk_size: usize,
        overlap: usize,
        metadata: &DocMetadata,
    ) -> Result<(), String> {
        let engine = self.engine.as_ref().ok_or("embedding engine not loaded")?;
        let chunks = crate::rag::chunker::chunk_text(text, chunk_size, overlap);
        if chunks.is_empty() {
            return Err("no content to index".into());
        }

        // Heavy work (embedding) happens outside the DB lock.
        let mut embedded: Vec<(String, Vec<f32>)> = Vec::with_capacity(chunks.len());
        {
            let engine = engine.lock().unwrap();
            for chunk in &chunks {
                let vec = engine.embed(chunk);
                embedded.push((chunk.clone(), vec));
            }
        }

        let id = format!("doc_{}", chrono::Utc::now().timestamp_millis());
        let created_at = chrono::Utc::now().to_rfc3339();
        let title = title.to_string();

        self.db.with_conn(|c| {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO documents (id, title, created_at, source_type, source_url) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    title,
                    created_at,
                    metadata.source_type,
                    metadata.source_url
                ],
            )?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO chunks (doc_id, idx, text, embedding) VALUES (?1, ?2, ?3, ?4)",
                )?;
                let mut fts =
                    tx.prepare("INSERT INTO chunks_fts (doc_id, text) VALUES (?1, ?2)")?;
                for (i, (chunk_text, vec)) in embedded.iter().enumerate() {
                    stmt.execute(params![id, i as i64, chunk_text, embed_to_blob(vec)])?;
                    fts.execute(params![id, chunk_text])?;
                }
            }
            tx.commit()
        })
    }

    /// 删除文档及其全部分块与 FTS 行。
    ///
    /// 单事务内先显式删 `chunks_fts` 中该 `doc_id` 的行，再删 `documents`；
    /// `chunks` 行由外键 `ON DELETE CASCADE`（需 `foreign_keys=ON`）随之清除。
    /// 显式删 FTS 是必需的：`chunks_fts` 是独立表，不参与该级联。
    pub(crate) fn delete_document(&self, id: &str) -> Result<(), String> {
        self.db.with_conn(|c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM chunks_fts WHERE doc_id = ?1", params![id])?;
            tx.execute("DELETE FROM documents WHERE id = ?1", params![id])?;
            tx.commit()
        })
    }

    /// 向量检索：对查询求 embedding，再对全部分块按余弦相似度排序取前 `top_k`。
    ///
    /// 契约：须先装入嵌入后端，否则返回错误。内部全量加载文档计算，适合中小知识库。
    /// 与 [`VectorStore::search_text`]（FTS5 关键词）是并存的另一条检索路径。
    #[allow(dead_code)]
    pub(crate) fn search(&self, query: &str, top_k: usize) -> Result<Vec<SearchHit>, String> {
        let engine = self.engine.as_ref().ok_or("embedding engine not loaded")?;
        let query_vec = engine.lock().unwrap().embed(query);
        let docs = self.documents();
        Ok(search_by_vector(&docs, &query_vec, top_k))
    }

    /// 对查询文本求 embedding 向量。
    ///
    /// 契约：须先装入嵌入后端，否则返回错误；与索引期共用同一把引擎锁。
    pub(crate) fn embed_query(&self, query: &str) -> Result<Vec<f32>, String> {
        let engine = self.engine.as_ref().ok_or("embedding engine not loaded")?;
        Ok(engine.lock().unwrap().embed(query))
    }

    /// [`VectorStore::documents`] 的语义别名，用于对外暴露只读快照。
    pub(crate) fn documents_snapshot(&self) -> Vec<StoredDocument> {
        self.documents()
    }

    /// Full-text keyword search over indexed chunks (FTS5).
    ///
    /// 用户输入里的 FTS5 查询语法元字符会被当作语法而非检索词，导致
    /// 语法错误或语义错位（`a:b` 被解释成列过滤、`AND` 被解释成布尔
    /// 运算符、`^` 被解释成列锚定）。这里把元字符统一替换为空格：
    /// 既保证任何输入都不会报错，又保留分词效果（`GPT-4` → `GPT 4`，
    /// 与内容侧 unicode61 的分词结果一致）。
    ///
    /// 结果经 `JOIN documents` 过滤：`chunks_fts` 可能残留文档已删的孤儿行，
    /// 该 INNER JOIN 是挡住孤儿内容的承重墙——一旦缺失，已删文档的关键词命中
    /// 会重新出现在用户视野里（存储层由 [`rebuild_fts_if_stale`] 清理孤儿，
    /// 但查询侧仍须自保）。
    ///
    /// 另注：`unicode61` 把连续 CJK 串视作单个 token，故以空格分隔的中文词组
    /// 更易命中（测试数据据此用空格分词）。
    pub(crate) fn search_text(&self, query: &str, top_k: usize) -> Result<Vec<SearchHit>, String> {
        /// FTS5 查询语法中的元字符。
        const META: &[char] = &['"', '*', '(', ')', '^', ':', '{', '}', '[', ']', '-', '+'];

        let mut sanitized = String::with_capacity(query.len());
        for ch in query.chars() {
            sanitized.push(if META.contains(&ch) { ' ' } else { ch });
        }
        // 小写化顺带中和裸运算符：FTS5 只把大写的 AND/OR/NOT/NEAR 当运算符，
        // 小写后按普通词处理。unicode61 分词器本身大小写不敏感，故不影响结果。
        let q = sanitized.to_lowercase();
        let q = q.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        self.db.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT f.doc_id, d.title, f.text, d.source_type, d.source_url, bm25(chunks_fts) AS rank
                 FROM chunks_fts f
                 JOIN documents d ON d.id = f.doc_id
                 WHERE chunks_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![q, top_k as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })?;
            let mut hits = Vec::new();
            for row in rows {
                let (doc_id, title, text, source_type, source_url) = row?;
                let _ = doc_id;
                hits.push(SearchHit {
                    chunk: text,
                    // Keyword hits carry no similarity score.
                    score: None,
                    source: title,
                    metadata: DocMetadata {
                        source_type,
                        source_url,
                    },
                });
            }
            Ok(hits)
        })
    }
}

/// 读取全部文档及其分块（按 `created_at`、`idx` 排序），并把 embedding
/// BLOB 还原为向量。
fn load_all_documents(c: &Connection) -> rusqlite::Result<Vec<StoredDocument>> {
    let mut stmt = c.prepare(
        "SELECT id, title, created_at, source_type, source_url \
         FROM documents ORDER BY created_at",
    )?;
    let rows: Vec<(String, String, String, String, Option<String>)> = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<Result<_, _>>()?;
    let mut docs = Vec::with_capacity(rows.len());
    for (id, title, created_at, source_type, source_url) in rows {
        let mut chunks = Vec::new();
        {
            let mut stmt =
                c.prepare("SELECT text, embedding FROM chunks WHERE doc_id = ?1 ORDER BY idx")?;
            let rows = stmt.query_map(params![id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            })?;
            for row in rows {
                let (text, blob) = row?;
                chunks.push(StoredChunk {
                    text,
                    embedding: blob_to_embed(&blob),
                });
            }
        }
        docs.push(StoredDocument {
            id,
            title,
            chunks,
            created_at,
            metadata: DocMetadata {
                source_type,
                source_url,
            },
        });
    }
    Ok(docs)
}

/// v1 → v2 迁移：为 `documents` 补上来源元数据两列。
///
/// 逐列用 `PRAGMA table_info` 探测，缺哪列补哪列——这样即使上次迁移在两次
/// ALTER 之间中断，重跑也能收敛到 v2 而不会因重复列报错（幂等）。
fn add_source_columns_if_missing(c: &Connection) -> rusqlite::Result<()> {
    fn column_exists(c: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
        let mut stmt = c.prepare(&format!("PRAGMA table_info({})", table))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
    }
    if !column_exists(c, "documents", "source_type")? {
        c.execute(
            "ALTER TABLE documents ADD COLUMN source_type TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    if !column_exists(c, "documents", "source_url")? {
        c.execute("ALTER TABLE documents ADD COLUMN source_url TEXT", [])?;
    }
    Ok(())
}

/// Import the legacy `vector_store.json` (if present) into the SQLite store.
fn migrate_from_json(store_dir: &std::path::Path, c: &mut Connection) -> Result<(), String> {
    let legacy_path = store_dir.join("vector_store.json");
    if !legacy_path.exists() {
        return Ok(());
    }
    #[derive(Deserialize)]
    struct LegacyData {
        #[serde(default)]
        documents: Vec<LegacyDocument>,
    }
    #[derive(Deserialize)]
    struct LegacyDocument {
        id: String,
        title: String,
        #[serde(default)]
        chunks: Vec<LegacyChunk>,
        #[serde(default)]
        created_at: String,
    }
    #[derive(Deserialize)]
    struct LegacyChunk {
        text: String,
        embedding: Vec<f32>,
    }

    // 注意：遗留 JSON 文件位于 kb_dir()（data_dir/knowledge_base），
    // 而沙箱目录为 data_dir/sandbox，两者不在同一路径下，
    // 因此不强制使用沙箱读取，避免破坏现有迁移逻辑。
    let raw =
        std::fs::read_to_string(&legacy_path).map_err(|e| format!("read legacy kb: {}", e))?;
    let data: LegacyData =
        serde_json::from_str(&raw).map_err(|e| format!("parse legacy kb: {}", e))?;
    if data.documents.is_empty() {
        return Ok(());
    }

    let tx = c.transaction().map_err(|e| e.to_string())?;
    for doc in &data.documents {
        let created_at = if doc.created_at.is_empty() {
            "1970-01-01T00:00:00Z".to_string()
        } else {
            doc.created_at.clone()
        };
        tx.execute(
            "INSERT OR IGNORE INTO documents (id, title, created_at) VALUES (?1, ?2, ?3)",
            params![doc.id, doc.title, created_at],
        )
        .map_err(|e| e.to_string())?;
        let mut stmt = tx
            .prepare("INSERT INTO chunks (doc_id, idx, text, embedding) VALUES (?1, ?2, ?3, ?4)")
            .map_err(|e| e.to_string())?;
        for (i, chunk) in doc.chunks.iter().enumerate() {
            stmt.execute(params![
                doc.id,
                i as i64,
                chunk.text,
                embed_to_blob(&chunk.embedding)
            ])
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    log::info!(
        "migrated {} documents from vector_store.json",
        data.documents.len()
    );
    Ok(())
}

/// Bring the FTS index back in sync with `chunks` when they disagree.
///
/// 三种失同步都必须修复，只比计数不够：
/// 1. `chunks` 有行而 FTS 缺行（FTS 支持加入前的老数据、迁移导入）→ 全量回填；
/// 2. `chunks_fts` 有孤儿行（文档已删但 FTS 行残留）→ 孤儿行会毒化计数启发式：
///    一条孤儿 + 一条漏索引恰好让计数相等，跳过本该发生的重建；且 `chunks`
///    为空时旧逻辑直接早退，孤儿 FTS 行永久残留（当前只是被 `search_text`
///    的 INNER JOIN 挡住才不可见——那面 JOIN 是承重墙，这里做存储层清理）；
/// 3. `chunks` 有孤儿行（历史库曾在 foreign_keys=OFF 下删文档、CASCADE 未
///    触发的残留）→ 直接从 chunks 回读重建会把已删内容重新灌回索引，且死行
///    永不清理。
///
/// 因此：发现任一孤儿或计数不一致时，在单个事务内先自愈删除两类孤儿行，
/// 再从活文档的 chunks 全量重建 FTS。幂等；健康库只花四次 COUNT。
fn rebuild_fts_if_stale(c: &mut Connection) -> rusqlite::Result<()> {
    let chunk_count: i64 = c.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
    let fts_count: i64 = c.query_row("SELECT COUNT(*) FROM chunks_fts", [], |r| r.get(0))?;
    let orphan_fts: i64 = c.query_row(
        "SELECT COUNT(*) FROM chunks_fts WHERE doc_id NOT IN (SELECT id FROM documents)",
        [],
        |r| r.get(0),
    )?;
    let orphan_chunks: i64 = c.query_row(
        "SELECT COUNT(*) FROM chunks WHERE doc_id NOT IN (SELECT id FROM documents)",
        [],
        |r| r.get(0),
    )?;
    if orphan_chunks == 0 && orphan_fts == 0 && (chunk_count == 0 || chunk_count == fts_count) {
        return Ok(());
    }
    log::info!(
        "rebuilding FTS index ({} chunks, {} indexed, {} orphaned FTS rows, {} orphaned chunks)",
        chunk_count,
        fts_count,
        orphan_fts,
        orphan_chunks
    );
    let tx = c.transaction()?;
    // 自愈 1：清理无主 chunks 行（历史 foreign_keys=OFF 期间 DELETE 的残留）。
    tx.execute(
        "DELETE FROM chunks WHERE doc_id NOT IN (SELECT id FROM documents)",
        [],
    )?;
    // 自愈 2 + 全量重建：清空 FTS 后只从活文档的 chunks 回填。
    tx.execute("DELETE FROM chunks_fts", [])?;
    {
        let mut select = tx.prepare("SELECT doc_id, text FROM chunks")?;
        let mut insert = tx.prepare("INSERT INTO chunks_fts (doc_id, text) VALUES (?1, ?2)")?;
        let rows =
            select.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (doc_id, text) = row?;
            insert.execute(params![doc_id, text])?;
        }
    }
    tx.commit()
}

/// 计算两向量的余弦相似度，值域 -1..1；任一向量模为零（或长度为零）时返回 0.0。
///
/// 按较短向量对齐（`zip` 截断），不要求两向量等长。
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let (dot, na, nb) = a
        .iter()
        .zip(b.iter())
        .take(n)
        .fold((0.0f32, 0.0f32, 0.0f32), |(d, a2, b2), (x, y)| {
            (d + x * y, a2 + x * x, b2 + y * y)
        });
    if na <= 0.0 || nb <= 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// 对给定文档集合按与 `query_vec` 的余弦相似度降序排序，返回前 `top_k` 条命中。
///
/// 纯内存计算、不触库；`source` 取文档标题（超过 40 字符截断并加省略号）。
/// 与 FTS 关键词路径（`search_text`）是并存的另一条检索路径。
pub(crate) fn search_by_vector(
    docs: &[StoredDocument],
    query_vec: &[f32],
    top_k: usize,
) -> Vec<SearchHit> {
    let mut scored: Vec<(String, f32, String, DocMetadata)> = Vec::new();
    for doc in docs {
        let source = if doc.title.len() > 40 {
            format!("{}...", &doc.title[..40])
        } else {
            doc.title.clone()
        };
        for chunk in &doc.chunks {
            let sim = cosine_similarity(query_vec, &chunk.embedding);
            scored.push((
                chunk.text.clone(),
                sim,
                source.clone(),
                doc.metadata.clone(),
            ));
        }
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);
    scored
        .into_iter()
        .map(|(chunk, score, source, metadata)| SearchHit {
            chunk,
            score: Some(score),
            source,
            metadata,
        })
        .collect()
}

/// RRF 融合常数：`k = 60` 抑制名次差距的过度放大（信息检索界常用取值）。
const RRF_K: f32 = 60.0;

/// 混合检索：向量命中（cosine，0..1）与关键词命中（FTS5 顺序）按 RRF 融合。
///
/// RRF（Reciprocal Rank Fusion）只看名次、不看原始分数量纲，因此能把余弦相似度
/// 与 FTS5 的 bm25 排序放在同一把尺子上比较：每路第 `rank`（从 1 开始）名贡献
/// `weight / (k + rank)`，本实现两路权重均为 1.0、`k = 60`，总分高者排前。
///
/// 去重键是 `chunk` 文本：同一分块可能同时被向量与关键词命中，此时把两路贡献
/// **相加**为一条命中，避免同一段内容在上下文里重复出现。输出 `score` 为 RRF
/// 融合分（非余弦值），`source`/`metadata` 取先出现的那一路的值。
///
/// 纯函数：不触库、不嵌入；任一路为空或两路都为空都能正确工作（空进空出）。
pub(crate) fn hybrid_search(
    vector_hits: Vec<SearchHit>,
    text_hits: Vec<SearchHit>,
    top_k: usize,
) -> Vec<SearchHit> {
    use std::collections::HashMap;

    /// 融合中间态：累计 RRF 分、首次出现顺序（用于稳定排序）与原始命中。
    struct Fused {
        score: f32,
        order: usize,
        hit: SearchHit,
    }

    let mut merged: HashMap<String, Fused> = HashMap::new();
    let mut next_order = 0usize;
    // 先向量路后关键词路：同分时向量路先出现（排序更稳定）。
    for hits in [vector_hits, text_hits] {
        for (i, hit) in hits.into_iter().enumerate() {
            let contribution = 1.0 / (RRF_K + (i + 1) as f32);
            if let Some(entry) = merged.get_mut(&hit.chunk) {
                entry.score += contribution;
            } else {
                merged.insert(
                    hit.chunk.clone(),
                    Fused {
                        score: contribution,
                        order: next_order,
                        hit,
                    },
                );
                next_order += 1;
            }
        }
    }

    let mut fused: Vec<Fused> = merged.into_values().collect();
    fused.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.order.cmp(&b.order))
    });
    fused.truncate(top_k);
    fused
        .into_iter()
        .map(|f| SearchHit {
            score: Some(f.score),
            ..f.hit
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (VectorStore, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = VectorStore::new(dir.path());
        (store, dir)
    }

    #[test]
    fn blob_roundtrip() {
        let v = vec![0.1f32, -0.5, std::f32::consts::PI, 0.0, 1e-8];
        let blob = embed_to_blob(&v);
        let back = blob_to_embed(&blob);
        assert_eq!(v, back);
    }

    #[test]
    fn empty_store_has_no_documents() {
        let (store, _dir) = temp_store();
        assert!(store.documents().is_empty());
        drop(store); // 先关闭 SQLite 句柄，TempDir 才能删除目录（Windows）
    }

    #[test]
    fn add_and_delete_document_without_engine_is_error() {
        let (store, _dir) = temp_store();
        assert!(store
            .add_document("t", "body", 500, 50, &DocMetadata::default())
            .is_err());
        drop(store);
    }

    #[test]
    fn search_ranking_matches_naive_expectation() {
        let docs = vec![StoredDocument {
            id: "a".into(),
            title: "文档A".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            metadata: DocMetadata::default(),
            chunks: vec![
                StoredChunk {
                    text: "苹果".into(),
                    embedding: vec![1.0, 0.0, 0.0],
                },
                StoredChunk {
                    text: "香蕉".into(),
                    embedding: vec![0.0, 1.0, 0.0],
                },
            ],
        }];
        let hits = search_by_vector(&docs, &[1.0, 0.0, 0.0], 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk, "苹果");
        assert!(matches!(hits[0].score, Some(s) if (s - 1.0).abs() < 1e-6));
    }

    #[test]
    fn search_topk_and_title_truncation() {
        let long_title = "x".repeat(60);
        let docs = vec![StoredDocument {
            id: "a".into(),
            title: long_title.clone(),
            created_at: "2026-01-01T00:00:00Z".into(),
            metadata: DocMetadata::default(),
            chunks: vec![StoredChunk {
                text: "t".into(),
                embedding: vec![1.0, 0.0],
            }],
        }];
        let hits = search_by_vector(&docs, &[0.0, 1.0], 1);
        assert_eq!(hits[0].source.len(), 43);
        assert!(hits[0].source.ends_with("..."));
    }

    // ── hybrid_search（RRF 融合） ──

    /// 构造向量路命中（带 cosine 分数）。
    fn vec_hit(chunk: &str, source: &str) -> SearchHit {
        SearchHit {
            chunk: chunk.into(),
            score: Some(0.5),
            source: source.into(),
            metadata: DocMetadata::default(),
        }
    }

    /// 构造关键词路命中（FTS5 无相似度分数）。
    fn kw_hit(chunk: &str, source: &str) -> SearchHit {
        SearchHit {
            chunk: chunk.into(),
            score: None,
            source: source.into(),
            metadata: DocMetadata::default(),
        }
    }

    #[test]
    fn hybrid_search_merges_duplicate_chunk_and_sums_scores() {
        let vector = vec![vec_hit("alpha", "向量源"), vec_hit("beta", "向量源")];
        let text = vec![kw_hit("beta", "关键词源"), kw_hit("gamma", "关键词源")];
        let fused = hybrid_search(vector, text, 10);

        assert_eq!(fused.len(), 3, "重复 chunk 必须合并为一条");
        // beta 两路都命中：1/62 + 1/61，应压过只有向量路 1/61 的 alpha。
        assert_eq!(fused[0].chunk, "beta");
        let beta_expected = 1.0 / 61.0 + 1.0 / 62.0;
        assert!(matches!(fused[0].score, Some(s) if (s - beta_expected).abs() < 1e-6));
        // source 与 metadata 取先出现的那一路（向量路）。
        assert_eq!(fused[0].source, "向量源");
        assert_eq!(fused[1].chunk, "alpha");
        assert!(matches!(fused[1].score, Some(s) if (s - 1.0 / 61.0).abs() < 1e-6));
        assert_eq!(fused[2].chunk, "gamma");
        assert!(matches!(fused[2].score, Some(s) if (s - 1.0 / 62.0).abs() < 1e-6));
    }

    #[test]
    fn hybrid_search_vector_only_orders_by_rank() {
        let vector = vec![
            vec_hit("first", "s"),
            vec_hit("second", "s"),
            vec_hit("third", "s"),
        ];
        let fused = hybrid_search(vector, Vec::new(), 3);
        let chunks: Vec<&str> = fused.iter().map(|h| h.chunk.as_str()).collect();
        assert_eq!(chunks, ["first", "second", "third"]);
        assert!(matches!(fused[0].score, Some(s) if (s - 1.0 / 61.0).abs() < 1e-6));
        assert!(matches!(fused[2].score, Some(s) if (s - 1.0 / 63.0).abs() < 1e-6));
    }

    #[test]
    fn hybrid_search_text_only_orders_by_rank() {
        let text = vec![kw_hit("a", "s"), kw_hit("b", "s"), kw_hit("c", "s")];
        let fused = hybrid_search(Vec::new(), text, 3);
        let chunks: Vec<&str> = fused.iter().map(|h| h.chunk.as_str()).collect();
        assert_eq!(chunks, ["a", "b", "c"]);
        assert!(matches!(fused[0].score, Some(s) if (s - 1.0 / 61.0).abs() < 1e-6));
        assert!(fused.iter().all(|h| h.score.is_some()));
    }

    #[test]
    fn hybrid_search_truncates_to_top_k() {
        let vector = vec![vec_hit("v1", "s"), vec_hit("v2", "s"), vec_hit("v3", "s")];
        let text = vec![kw_hit("t1", "s"), kw_hit("t2", "s")];
        let fused = hybrid_search(vector, text, 2);
        assert_eq!(fused.len(), 2, "top_k 必须截断融合结果");
    }

    #[test]
    fn hybrid_search_empty_inputs_return_empty() {
        assert!(hybrid_search(Vec::new(), Vec::new(), 5).is_empty());
        assert!(hybrid_search(vec![vec_hit("x", "s")], Vec::new(), 0).is_empty());
    }

    #[test]
    fn legacy_json_migration() {
        let dir = tempfile::TempDir::new().unwrap();
        let json = r#"{
            "documents": [{
                "id": "doc_old_1",
                "title": "旧文档",
                "created_at": "2025-01-01T00:00:00Z",
                "chunks": [{
                    "text": "旧文本",
                    "embedding": [1.0, 0.0, 0.0]
                }]
            }]
        }"#;
        std::fs::write(dir.path().join("vector_store.json"), json).unwrap();
        {
            let store = VectorStore::new(dir.path());
            let docs = store.documents();
            assert_eq!(docs.len(), 1);
            assert_eq!(docs[0].id, "doc_old_1");
            assert_eq!(docs[0].chunks.len(), 1);
            assert_eq!(docs[0].chunks[0].text, "旧文本");
            assert_eq!(docs[0].chunks[0].embedding, vec![1.0, 0.0, 0.0]);
        }
        // 幂等：迁移由 user_version 门控，再次打开不得重复导入
        {
            let store2 = VectorStore::new(dir.path());
            assert_eq!(store2.documents().len(), 1, "迁移重复执行产生了重复文档");
        }
    }

    /// v1 形态的库（只有 id/title/created_at 三列）重开时必须自动补上
    /// source_type/source_url 两列，老数据取默认值，且再次重开保持幂等。
    #[test]
    fn schema_v2_migration_adds_metadata_columns() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("kb.db");
        {
            let db = crate::store::db::open(&path).unwrap();
            db.with_conn(|c| {
                c.execute_batch(
                    "CREATE TABLE documents (
                        id         TEXT PRIMARY KEY,
                        title      TEXT NOT NULL,
                        created_at TEXT NOT NULL
                    );",
                )?;
                c.execute(
                    "INSERT INTO documents (id, title, created_at) \
                     VALUES ('old', '旧库文档', '2025-01-01T00:00:00Z')",
                    [],
                )?;
                c.pragma_update(None, "user_version", 1)?;
                Ok(())
            })
            .unwrap();
        }

        // 重开触发 v1 → v2 迁移。
        {
            let store = VectorStore::new(dir.path());
            let (columns, source_type, source_url, version) = store
                .db
                .with_conn(|c| {
                    let mut stmt = c.prepare("PRAGMA table_info(documents)")?;
                    let columns: Vec<String> = stmt
                        .query_map([], |r| r.get::<_, String>(1))?
                        .collect::<Result<_, _>>()?;
                    let (source_type, source_url): (String, Option<String>) = c.query_row(
                        "SELECT source_type, source_url FROM documents WHERE id = 'old'",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )?;
                    let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
                    Ok((columns, source_type, source_url, version))
                })
                .unwrap();
            assert!(columns.iter().any(|c| c == "source_type"));
            assert!(columns.iter().any(|c| c == "source_url"));
            assert_eq!(source_type, "", "老行 source_type 应为空串（未知来源）");
            assert_eq!(source_url, None, "老行 source_url 应为 NULL");
            assert_eq!(version, 2);
        }

        // 幂等：再次重开不报错，数据保持不变。
        {
            let store = VectorStore::new(dir.path());
            let docs = store.documents();
            assert_eq!(docs.len(), 1);
            assert_eq!(docs[0].metadata.source_type, "");
            assert_eq!(docs[0].metadata.source_url, None);
            let version: i64 = store
                .db
                .with_conn(|c| c.query_row("PRAGMA user_version", [], |r| r.get(0)))
                .unwrap();
            assert_eq!(version, 2);
        }
    }

    /// 元数据经三条读取路径（documents / FTS 检索 / 向量检索）往返一致。
    #[test]
    fn metadata_roundtrip_through_search_paths() {
        let (store, _dir) = temp_store();
        store
            .db
            .with_conn(|c| {
                c.execute(
                    "INSERT INTO documents (id, title, created_at, source_type, source_url) \
                     VALUES ('d_meta', '网页文档', '2026-02-01', 'crawl', 'https://example.com/x')",
                    [],
                )?;
                c.execute(
                    "INSERT INTO chunks (doc_id, idx, text, embedding) \
                     VALUES ('d_meta', 0, '元数据 往返 校验 关键词', ?1)",
                    params![embed_to_blob(&[1.0f32, 0.0, 0.0])],
                )?;
                c.execute(
                    "INSERT INTO chunks_fts (doc_id, text) \
                     VALUES ('d_meta', '元数据 往返 校验 关键词')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();

        let docs = store.documents();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].metadata.source_type, "crawl");
        assert_eq!(
            docs[0].metadata.source_url.as_deref(),
            Some("https://example.com/x")
        );

        let hits = store.search_text("元数据", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].metadata.source_type, "crawl");
        assert_eq!(
            hits[0].metadata.source_url.as_deref(),
            Some("https://example.com/x")
        );

        let vec_hits = search_by_vector(&docs, &[1.0, 0.0, 0.0], 5);
        assert_eq!(vec_hits.len(), 1);
        assert_eq!(vec_hits[0].metadata.source_type, "crawl");
        assert_eq!(
            vec_hits[0].metadata.source_url.as_deref(),
            Some("https://example.com/x")
        );
        drop(store);
    }

    #[test]
    fn fts5_module_available_in_bundled_sqlite() {
        let (store, _dir) = temp_store();
        let result = store.db.with_conn(|c| {
            c.execute_batch("CREATE VIRTUAL TABLE IF NOT EXISTS _fts_probe USING fts5(x)")
        });
        assert!(
            result.is_ok(),
            "bundled SQLite must support FTS5: {:?}",
            result
        );
        drop(store);
    }

    #[test]
    fn fts_backfill_indexes_existing_chunks() {
        let dir = tempfile::TempDir::new().unwrap();
        // Insert a chunk directly (bypassing add_document) to simulate data
        // that predates the FTS table, then reopen to trigger the backfill.
        {
            let store = VectorStore::new(dir.path());
            let _ = store.db.with_conn(|c| {
                c.execute(
                    "INSERT INTO documents (id, title, created_at) VALUES ('d1', '测试文档', '2026-01-01T00:00:00Z')",
                    [],
                )?;
                c.execute(
                    "INSERT INTO chunks (doc_id, idx, text, embedding) VALUES ('d1', 0, '含有关键词 苹果 的描述', x'00000000')",
                    [],
                )
            });
        }
        let store = VectorStore::new(dir.path());
        let hits = store.search_text("苹果", 5).expect("search_text");
        assert_eq!(hits.len(), 1, "backfilled chunk must be searchable");
        assert_eq!(hits[0].source, "测试文档");
        drop(store);
    }

    #[test]
    fn delete_document_purges_cascade_and_survives_fts_rebuild() {
        let dir = tempfile::TempDir::new().unwrap();
        {
            let store = VectorStore::new(dir.path());
            index_chunk_sql(&store, "d1", "待删文档", "甲辰独语 文档一号 保密内容");
            index_chunk_sql(&store, "d2", "保留文档", "香蕉面包 完全无关 食谱");
            assert_eq!(store.search_text("甲辰独语", 5).unwrap().len(), 1);
            assert_eq!(store.search_text("香蕉面包", 5).unwrap().len(), 1);

            store.delete_document("d1").unwrap();

            // ON DELETE CASCADE 不变量：chunks 行必须随文档删除而消失
            //（delete_document 不显式删 chunks，全靠 foreign_keys=ON）。
            let orphaned: i64 = store
                .db
                .with_conn(|c| {
                    c.query_row("SELECT COUNT(*) FROM chunks WHERE doc_id = 'd1'", [], |r| {
                        r.get(0)
                    })
                })
                .unwrap();
            assert_eq!(orphaned, 0, "CASCADE 必须清掉已删文档的 chunks 行");

            // 已删内容立刻不可检索；其余文档不受影响。
            assert!(store.search_text("甲辰独语", 5).unwrap().is_empty());
            assert_eq!(store.search_text("香蕉面包", 5).unwrap().len(), 1);
        }
        // 重开（触发 rebuild_fts_if_stale）：已删内容不得借全量重建复活。
        let store2 = VectorStore::new(dir.path());
        assert!(
            store2.search_text("甲辰独语", 5).unwrap().is_empty(),
            "rebuild 后已删内容不可检索"
        );
        assert_eq!(store2.search_text("香蕉面包", 5).unwrap().len(), 1);
        assert_eq!(store2.documents().len(), 1);
        drop(store2);
    }

    #[test]
    fn rebuild_fts_self_heals_orphaned_rows_despite_equal_counts() {
        let dir = tempfile::TempDir::new().unwrap();
        {
            let store = VectorStore::new(dir.path());
            index_chunk_sql(&store, "d1", "活文档", "正常内容 活文档 段落");
            // 模拟历史脏库：老版本在 foreign_keys=OFF 时 DELETE documents 不触发
            // CASCADE 留下的无主 chunks 行，以及中断删除留下的无主 FTS 行。
            // 计数恰好相等（2==2）——纯计数启发式会直接早退不修复。
            store
                .db
                .with_conn(|c| {
                    c.pragma_update(None, "foreign_keys", "OFF")?;
                    c.execute(
                        "INSERT INTO chunks (doc_id, idx, text, embedding) \
                         VALUES ('ghost_c', 0, '幽灵块 已删残留', x'00000000')",
                        [],
                    )?;
                    c.execute(
                        "INSERT INTO chunks_fts (doc_id, text) \
                         VALUES ('ghost_f', '幽灵行 已删残留')",
                        [],
                    )?;
                    c.pragma_update(None, "foreign_keys", "ON")?;
                    Ok(())
                })
                .unwrap();
        }
        let store2 = VectorStore::new(dir.path());
        let (chunks_total, fts_total, ghost_chunks): (i64, i64, i64) = store2
            .db
            .with_conn(|c| {
                Ok((
                    c.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?,
                    c.query_row("SELECT COUNT(*) FROM chunks_fts", [], |r| r.get(0))?,
                    c.query_row(
                        "SELECT COUNT(*) FROM chunks \
                         WHERE doc_id NOT IN (SELECT id FROM documents)",
                        [],
                        |r| r.get(0),
                    )?,
                ))
            })
            .unwrap();
        assert_eq!(ghost_chunks, 0, "无主 chunks 行应被自愈清除");
        assert_eq!(
            fts_total, 1,
            "无主 FTS 行应被清除，只留活文档的行（得到 {fts_total}）"
        );
        assert_eq!(chunks_total, 1);
        // 用户可见层面：幽灵内容不可检索，活文档照常可检索。
        assert!(store2.search_text("幽灵块", 5).unwrap().is_empty());
        assert!(store2.search_text("幽灵行", 5).unwrap().is_empty());
        assert_eq!(store2.search_text("正常内容", 5).unwrap().len(), 1);
        drop(store2);
    }

    #[test]
    fn rebuild_fts_purges_orphans_even_when_chunks_table_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        {
            let store = VectorStore::new(dir.path());
            index_chunk_sql(&store, "d1", "文档", "唯一内容 独占词条");
            store.delete_document("d1").unwrap(); // 干净清空：chunks=0, fts=0
            store
                .db
                .with_conn(|c| {
                    c.pragma_update(None, "foreign_keys", "OFF")?;
                    c.execute(
                        "INSERT INTO chunks_fts (doc_id, text) \
                         VALUES ('ghost', '复活 词条 残留')",
                        [],
                    )?;
                    c.pragma_update(None, "foreign_keys", "ON")?;
                    Ok(())
                })
                .unwrap();
        }
        // 旧逻辑在 chunks==0 时直接早退，孤儿 FTS 行会永久残留。
        let store2 = VectorStore::new(dir.path());
        let fts_total: i64 = store2
            .db
            .with_conn(|c| c.query_row("SELECT COUNT(*) FROM chunks_fts", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(fts_total, 0, "空库时孤儿 FTS 行也必须被清理");
        assert!(store2.search_text("复活", 5).unwrap().is_empty());
        assert!(store2.documents().is_empty());
        drop(store2);
    }

    #[test]
    fn search_text_quote_and_empty_safe() {
        let (store, _dir) = temp_store();
        // 引号被中和,不会语法错误
        let hits = store.search_text("a\"b", 5).expect("no crash");
        assert!(hits.is_empty());
        // 空查询返回空结果
        let hits = store.search_text("   ", 5).expect("no crash");
        assert!(hits.is_empty());
        drop(store);
    }

    #[test]
    fn persistence_across_reopen() {
        let dir = tempfile::TempDir::new().unwrap();
        {
            let store = VectorStore::new(dir.path());
            let _ = &store;
        }
        let store2 = VectorStore::new(dir.path());
        assert!(store2.documents().is_empty());
        drop(store2);
    }

    #[test]
    fn test_store_is_send_sync() {
        // 编译期断言：后台索引线程需要把 VectorStore 共享到其他线程。
        // EmbeddingEngine 本身非 Sync，靠 Mutex 包裹后 VectorStore 必须
        // 满足 Send + Sync 才能跨线程使用。
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<VectorStore>();
    }

    // ── RAG end-to-end: index -> retrieve -> budgeted prompt ──

    /// Insert a document + chunk + FTS row directly (the embedding engine
    /// needs a model and is not available in tests).
    fn index_chunk_sql(store: &VectorStore, doc_id: &str, title: &str, chunk: &str) {
        store
            .db
            .with_conn(|c| {
                c.execute(
                    "INSERT INTO documents (id, title, created_at, source_type, source_url) \
                     VALUES (?1, ?2, '2026-01-01', 'paste', NULL)",
                    params![doc_id, title],
                )?;
                let emb = embed_to_blob(&[0.0f32; 4]);
                c.execute(
                    "INSERT INTO chunks (doc_id, idx, text, embedding) VALUES (?1, 0, ?2, ?3)",
                    params![doc_id, chunk, emb],
                )?;
                c.execute(
                    "INSERT INTO chunks_fts (doc_id, text) VALUES (?1, ?2)",
                    params![doc_id, chunk],
                )?;
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn rag_pipeline_search_then_prompt_within_budget() {
        use crate::llm::inference::build_rag_prompt_budgeted;
        use crate::store::conversation::Message;

        let (store, _dir) = temp_store();
        index_chunk_sql(
            &store,
            "d1",
            "苹果种植手册",
            "苹果派 的做法：先准备新鲜苹果，去皮切块，加糖腌制。",
        );

        // 2. Retrieve via FTS5 (no model required).
        let hits = store.search_text("苹果派", 3).unwrap();
        assert!(!hits.is_empty(), "FTS should find the indexed chunk");

        // 3. Build the KB context exactly like the UI does.
        let mut kb_ctx = String::new();
        for (i, hit) in hits.iter().enumerate() {
            kb_ctx.push_str(&format!(
                "[参考{} 来源: {}]\n{}\n\n",
                i + 1,
                hit.source,
                hit.chunk
            ));
        }

        // 4. Assemble the budgeted prompt (stub counter: 1 char = 1 token).
        let msgs = vec![
            Message {
                role: "system".into(),
                content: "你是助手".into(),
            },
            Message {
                role: "user".into(),
                content: "苹果派怎么做？".into(),
            },
        ];
        let budget = 2000usize;
        let out = build_rag_prompt_budgeted(&msgs, Some(&kb_ctx), None, budget, |s: &str| {
            s.chars().count()
        });
        assert!(
            out.prompt.contains("苹果派") && out.prompt.contains("去皮切块"),
            "retrieved chunk must reach the prompt: {}",
            out.prompt
        );
        assert!(!out.over_budget);
        assert!(
            out.prompt.chars().count() <= budget,
            "prompt exceeds budget: {} > {}",
            out.prompt.chars().count(),
            budget
        );
        assert_eq!(out.dropped_messages, 0);
        drop(store);
    }

    #[test]
    fn rag_pipeline_truncates_history_but_keeps_kb_context() {
        use crate::llm::inference::build_rag_prompt_budgeted;
        use crate::store::conversation::Message;

        let (store, _dir) = temp_store();
        index_chunk_sql(&store, "d1", "手册", "重要参考：答案就在这一段里。");
        let hits = store.search_text("重要参考", 1).unwrap();
        assert!(!hits.is_empty());
        let kb_ctx = hits[0].chunk.clone();

        // Long history: 20 turns of old chatter + the final question.
        let mut msgs = vec![Message {
            role: "system".into(),
            content: "你是助手".into(),
        }];
        for i in 0..20 {
            msgs.push(Message {
                role: "user".into(),
                content: format!("很长的旧消息{}号，包含着大量的历史内容需要被截断。", i),
            });
            msgs.push(Message {
                role: "assistant".into(),
                content: format!("旧回复{}号。", i),
            });
        }
        msgs.push(Message {
            role: "user".into(),
            content: "最终问题".into(),
        });

        // Budget: enough for system + kb + final question + a couple of
        // recent turns, not the whole history.
        let budget = 260usize;
        let out = build_rag_prompt_budgeted(&msgs, Some(&kb_ctx), None, budget, |s: &str| {
            s.chars().count()
        });
        assert!(out.dropped_messages > 0, "history should be truncated");
        assert!(
            out.prompt.contains("重要参考"),
            "KB context lives in the system prompt and must survive"
        );
        assert!(out.prompt.contains("最终问题"), "final question preserved");
        assert!(
            !out.prompt.contains("消息0号"),
            "oldest history should be dropped: {}",
            out.prompt
        );
        assert!(out.prompt.chars().count() <= budget);
        drop(store);
    }
}
