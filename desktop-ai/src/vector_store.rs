use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VectorStoreData {
    documents: Vec<StoredDocument>,
}

pub struct VectorStore {
    path: PathBuf,
    data: VectorStoreData,
    engine: Option<EmbeddingEngine>,
}

impl VectorStore {
    pub fn new(store_dir: &std::path::Path) -> Self {
        let path = store_dir.join("vector_store.json");
        let data = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(VectorStoreData {
                    documents: Vec::new(),
                })
        } else {
            VectorStoreData {
                documents: Vec::new(),
            }
        };
        Self {
            path,
            data,
            engine: None,
        }
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
        self.engine = Some(engine);
    }

    pub fn has_engine(&self) -> bool {
        self.engine.is_some()
    }

    pub fn documents(&self) -> &[StoredDocument] {
        &self.data.documents
    }

    pub fn add_document(
        &mut self,
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

        let id = format!("doc_{}", chrono::Utc::now().timestamp_millis());
        let mut stored_chunks = Vec::new();

        for chunk in &chunks {
            let vec = engine.embed(chunk);
            stored_chunks.push(StoredChunk {
                text: chunk.clone(),
                embedding: vec,
            });
        }

        self.data.documents.push(StoredDocument {
            id,
            title: title.to_string(),
            chunks: stored_chunks,
            created_at: chrono::Utc::now().to_rfc3339(),
        });

        self.save()?;
        Ok(())
    }

    pub fn delete_document(&mut self, id: &str) -> Result<(), String> {
        self.data.documents.retain(|d| d.id != id);
        self.save()
    }

    #[allow(dead_code)]
    pub fn search(&self, query: &str, top_k: usize) -> Result<Vec<SearchHit>, String> {
        let engine = self.engine.as_ref().ok_or("embedding engine not loaded")?;
        let query_vec = engine.embed(query);
        Ok(search_by_vector(&self.data.documents, &query_vec, top_k))
    }

    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>, String> {
        let engine = self.engine.as_ref().ok_or("embedding engine not loaded")?;
        Ok(engine.embed(query))
    }

    pub fn documents_snapshot(&self) -> Vec<StoredDocument> {
        self.data.documents.clone()
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {}", e))?;
        }
        let json =
            serde_json::to_string_pretty(&self.data).map_err(|e| format!("serialize: {}", e))?;
        std::fs::write(&self.path, &json).map_err(|e| format!("write: {}", e))?;
        Ok(())
    }
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
