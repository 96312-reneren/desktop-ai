use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::db::Db;
use crate::embedding::EmbeddingEngine;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredChunk {
    pub text: String,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub chunk: String,
    pub score: f32,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredDocument {
    pub id: String,
    pub title: String,
    pub chunks: Vec<StoredChunk>,
    pub created_at: String,
}

/// SQLite-backed vector store.
///
/// Concurrency: embedding inference happens *before* taking the DB lock;
/// inside the lock only short SQL transactions run (see [`crate::db`]).
///
/// The embedding engine is guarded by a `Mutex`: `EmbeddingEngine` is
/// `Send` but not `Sync` (it owns raw FFI pointers), and indexing runs on a
/// background thread while the UI thread may call `embed_query` at the same
/// time. Both paths lock the same mutex, so inference is serialised and the
/// engine is only ever touched by one thread at a time. Keep heavy work
/// inside the lock to a minimum.
pub struct VectorStore {
    db: Db,
    engine: Option<Arc<Mutex<EmbeddingEngine>>>,
}

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS documents (
    id         TEXT PRIMARY KEY,
    title      TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS chunks (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    doc_id    TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    idx       INTEGER NOT NULL,
    text      TEXT NOT NULL,
    embedding BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_chunks_doc ON chunks(doc_id);
";

fn embed_to_blob(v: &[f32]) -> Vec<u8> {
    let mut bytes = vec![0u8; v.len() * 4];
    for (i, x) in v.iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&x.to_le_bytes());
    }
    bytes
}

fn blob_to_embed(bytes: &[u8]) -> Vec<f32> {
    let mut out = vec![0f32; bytes.len() / 4];
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.as_mut_ptr() as *mut u8, bytes.len());
    }
    out
}

impl VectorStore {
    pub fn new(store_dir: &std::path::Path) -> Self {
        let db = crate::db::open(&store_dir.join("kb.db")).unwrap_or_else(|e| {
            log::error!("failed to open kb.db: {} — using in-memory fallback", e);
            crate::db::open(
                &std::env::temp_dir()
                    .join(format!("desktop_ai_kb_fallback_{}.db", std::process::id())),
            )
            .expect("fallback kb db")
        });
        if let Err(e) = db.with_conn(|c| {
            c.execute_batch(SCHEMA)?;
            let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            if version < SCHEMA_VERSION {
                if let Err(e) = migrate_from_json(store_dir, c) {
                    log::error!("kb JSON migration failed: {}", e);
                }
                c.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            }
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
    pub fn set_engine(&mut self, engine: EmbeddingEngine) {
        self.engine = Some(Arc::new(Mutex::new(engine)));
    }

    pub fn has_engine(&self) -> bool {
        self.engine.is_some()
    }

    pub fn documents(&self) -> Vec<StoredDocument> {
        self.db
            .with_conn(|c| load_all_documents(c))
            .unwrap_or_default()
    }

    pub fn add_document(
        &self,
        title: &str,
        text: &str,
        chunk_size: usize,
        overlap: usize,
    ) -> Result<(), String> {
        let engine = self.engine.as_ref().ok_or("embedding engine not loaded")?;
        let chunks = crate::chunker::chunk_text(text, chunk_size, overlap);
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
                "INSERT INTO documents (id, title, created_at) VALUES (?1, ?2, ?3)",
                params![id, title, created_at],
            )?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO chunks (doc_id, idx, text, embedding) VALUES (?1, ?2, ?3, ?4)",
                )?;
                for (i, (chunk_text, vec)) in embedded.iter().enumerate() {
                    stmt.execute(params![id, i as i64, chunk_text, embed_to_blob(vec)])?;
                }
            }
            tx.commit()
        })
    }

    pub fn delete_document(&self, id: &str) -> Result<(), String> {
        self.db.with_conn(|c| {
            c.execute("DELETE FROM documents WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    #[allow(dead_code)]
    pub fn search(&self, query: &str, top_k: usize) -> Result<Vec<SearchHit>, String> {
        let engine = self.engine.as_ref().ok_or("embedding engine not loaded")?;
        let query_vec = engine.lock().unwrap().embed(query);
        let docs = self.documents();
        Ok(search_by_vector(&docs, &query_vec, top_k))
    }

    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>, String> {
        let engine = self.engine.as_ref().ok_or("embedding engine not loaded")?;
        Ok(engine.lock().unwrap().embed(query))
    }

    pub fn documents_snapshot(&self) -> Vec<StoredDocument> {
        self.documents()
    }
}

fn load_all_documents(c: &Connection) -> rusqlite::Result<Vec<StoredDocument>> {
    let mut stmt = c.prepare("SELECT id, title, created_at FROM documents ORDER BY created_at")?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let mut docs = Vec::with_capacity(rows.len());
    for (id, title, created_at) in rows {
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
        });
    }
    Ok(docs)
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

pub fn search_by_vector(
    docs: &[StoredDocument],
    query_vec: &[f32],
    top_k: usize,
) -> Vec<SearchHit> {
    let mut scored: Vec<(String, f32, String)> = Vec::new();
    for doc in docs {
        let source = if doc.title.len() > 40 {
            format!("{}...", &doc.title[..40])
        } else {
            doc.title.clone()
        };
        for chunk in &doc.chunks {
            let sim = cosine_similarity(query_vec, &chunk.embedding);
            scored.push((chunk.text.clone(), sim, source.clone()));
        }
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);
    scored
        .into_iter()
        .map(|(chunk, score, source)| SearchHit {
            chunk,
            score,
            source,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (VectorStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "desktop_ai_kb_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = VectorStore::new(&dir);
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
        let (store, dir) = temp_store();
        assert!(store.documents().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn add_and_delete_document_without_engine_is_error() {
        let (store, dir) = temp_store();
        assert!(store.add_document("t", "body", 500, 50).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn search_ranking_matches_naive_expectation() {
        let docs = vec![StoredDocument {
            id: "a".into(),
            title: "文档A".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
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
        assert!((hits[0].score - 1.0).abs() < 1e-6);
    }

    #[test]
    fn search_topk_and_title_truncation() {
        let long_title = "x".repeat(60);
        let docs = vec![StoredDocument {
            id: "a".into(),
            title: long_title.clone(),
            created_at: "2026-01-01T00:00:00Z".into(),
            chunks: vec![StoredChunk {
                text: "t".into(),
                embedding: vec![1.0, 0.0],
            }],
        }];
        let hits = search_by_vector(&docs, &[0.0, 1.0], 1);
        assert_eq!(hits[0].source.len(), 43);
        assert!(hits[0].source.ends_with("..."));
    }

    #[test]
    fn legacy_json_migration() {
        let dir = std::env::temp_dir().join(format!(
            "desktop_ai_kb_mig_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
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
        std::fs::write(dir.join("vector_store.json"), json).unwrap();
        let store = VectorStore::new(&dir);
        let docs = store.documents();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, "doc_old_1");
        assert_eq!(docs[0].chunks.len(), 1);
        assert_eq!(docs[0].chunks[0].text, "旧文本");
        assert_eq!(docs[0].chunks[0].embedding, vec![1.0, 0.0, 0.0]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn persistence_across_reopen() {
        let dir = std::env::temp_dir().join(format!(
            "desktop_ai_kb_persist_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let store = VectorStore::new(&dir);
            let _ = &store;
        }
        let store2 = VectorStore::new(&dir);
        assert!(store2.documents().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_store_is_send_sync() {
        // 编译期断言：后台索引线程需要把 VectorStore 共享到其他线程。
        // EmbeddingEngine 本身非 Sync，靠 Mutex 包裹后 VectorStore 必须
        // 满足 Send + Sync 才能跨线程使用。
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<VectorStore>();
    }
}
