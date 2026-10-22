//! 知识库后台索引流水线。
//!
//! 从 UI 模块迁出（可维护性评审 #16）：这是一套后台任务状态机——读取 /
//! 爬取、分块、向量化、入库，通过 channel 上报进度——与界面唯一的耦合是
//! "把进度画出来"。迁到 rag 领域后这条链路可以独立测试。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use super::crawler::extract_pdf_safe;
use super::vector_store::VectorStore;

/// 知识库文本文件大小上限（64 MiB）。超限文件在 read_to_string 之前即被
/// 拒绝，避免把整个文件一次性读进内存（正文入库另有 KB_SINGLE_DOC_CHARS
/// 分段，此限只防无界分配，不影响正常文档）。
pub(crate) const MAX_KB_TEXT_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// 知识库 PDF 大小上限（256 MiB）：pdf-extract 会把整份 PDF 载入内存。
pub(crate) const MAX_KB_PDF_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// 导入对话 JSON 的大小上限（64 MiB），供 ui::app::import_conversation 复用。
pub(crate) const MAX_IMPORT_JSON_BYTES: u64 = 64 * 1024 * 1024;

/// 读取前大小预检：文件超过 `max_bytes` 时返回用户可读的错误信息。
/// 先 metadata 再读取，保证超大文件不会先被完整载入内存才失败。
pub(crate) fn ensure_file_size(path: &std::path::Path, max_bytes: u64) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > max_bytes => Err(format!(
            "文件过大: {:.1} MB（上限 {:.0} MB）: {}",
            m.len() as f64 / (1024.0 * 1024.0),
            max_bytes as f64 / (1024.0 * 1024.0),
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(e) => Err(format!("无法读取文件信息: {}: {}", path.display(), e)),
    }
}

/// 后台索引线程 → UI 线程的消息。
pub(crate) enum KbIndexMsg {
    Progress {
        frac: f32,
        status: String,
    },
    Done {
        status: String,
        status_message: String,
        clear_url: bool,
        clear_title: bool,
        clear_content: bool,
    },
    Error(String),
}

/// 待执行的索引任务：UI 线程只做参数准备与校验，重活在后台线程完成。
pub(crate) enum KbIndexJob {
    File {
        path: PathBuf,
        filename: String,
        ext: String,
    },
    Paste {
        title: String,
        content: String,
    },
    Crawl {
        url: String,
        depth: u32,
    },
}

/// 索引成功时的收尾信息：单文档状态的文案，以及 UI 需要清空的输入项。
struct KbDoneInfo {
    single_status: String,
    single_message: String,
    clear_url: bool,
    clear_title: bool,
    clear_content: bool,
}

/// 后台知识库索引线程入口：按任务类型执行读取/爬取、分块、向量化与入库，
/// 通过 `tx` 上报进度与结果；UI 线程由 `DesktopAI::poll_kb_job` 接收。
pub(crate) fn run_kb_job(
    store: Arc<VectorStore>,
    job: KbIndexJob,
    cancel: Arc<AtomicBool>,
    tx: mpsc::Sender<KbIndexMsg>,
) {
    match job {
        KbIndexJob::File {
            path,
            filename,
            ext,
        } => {
            // 大小预检：用户可选中任意本地文件（包括恶意构造的巨型文件），
            // 必须在 read_to_string / pdf-extract 之前拒绝超限文件。
            let max_bytes = if ext == "pdf" {
                MAX_KB_PDF_FILE_BYTES
            } else {
                MAX_KB_TEXT_FILE_BYTES
            };
            if let Err(e) = ensure_file_size(&path, max_bytes) {
                let _ = tx.send(KbIndexMsg::Error(e));
                return;
            }
            let content = match ext.as_str() {
                "pdf" => match extract_pdf_safe(&path) {
                    Ok(t) => t,
                    Err(e) => {
                        let _ = tx.send(KbIndexMsg::Error(format!("PDF解析失败: {}", e)));
                        return;
                    }
                },
                _ => match std::fs::read_to_string(&path) {
                    Ok(t) => t,
                    Err(e) => {
                        let _ = tx.send(KbIndexMsg::Error(format!("读取失败: {}", e)));
                        return;
                    }
                },
            };
            let _ = tx.send(KbIndexMsg::Progress {
                frac: 0.2,
                status: "分块中...".into(),
            });
            let result = index_content(&store, &filename, &content, 500, 50, &cancel, &tx);
            finish_kb_job(
                &tx,
                result,
                KbDoneInfo {
                    single_status: format!("已添加: {}", filename),
                    single_message: format!("已索引文档: {}", filename),
                    clear_url: false,
                    clear_title: false,
                    clear_content: false,
                },
            );
        }
        KbIndexJob::Paste { title, content } => {
            let char_count = content.chars().count();
            let _ = tx.send(KbIndexMsg::Progress {
                frac: 0.2,
                status: format!("分块中... ({:.0} 字符)", char_count as f64),
            });
            let result = index_content(&store, &title, &content, 512, 64, &cancel, &tx);
            finish_kb_job(
                &tx,
                result,
                KbDoneInfo {
                    single_status: "完成".into(),
                    single_message: format!("已添加文档: {}", title),
                    clear_url: false,
                    clear_title: true,
                    clear_content: true,
                },
            );
        }
        KbIndexJob::Crawl { url, depth } => {
            let _ = tx.send(KbIndexMsg::Progress {
                frac: 0.05,
                status: if depth > 1 {
                    format!("深度爬取(≤{}层): {}", depth, url)
                } else {
                    format!("正在爬取: {}", url)
                },
            });
            let results = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if depth > 1 {
                    let config = crate::rag::crawler::CrawlConfig {
                        max_depth: depth,
                        max_pages: 15,
                        ..Default::default()
                    };
                    crate::rag::crawler::crawl_with_depth(&url, config)
                } else {
                    vec![crate::rag::crawler::crawl_url(&url)]
                }
            }))
            .unwrap_or_else(|_| {
                // A panic inside the crawler must not kill the job silently.
                let _ = tx.send(KbIndexMsg::Error("爬取过程发生内部错误，已中止".into()));
                Vec::new()
            });

            let total = results.len();
            let mut added = 0usize;
            for result in &results {
                if cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(KbIndexMsg::Error("已取消".into()));
                    return;
                }
                match result {
                    Ok(page) => {
                        let _ = tx.send(KbIndexMsg::Progress {
                            frac: 0.3 + (added as f32 / total as f32) * 0.6,
                            status: format!(
                                "索引 {}/{}: {}",
                                added + 1,
                                total,
                                // char-boundary safe (was a byte slice; a
                                // Chinese title could panic)
                                page.title.chars().take(30).collect::<String>()
                            ),
                        });
                        if let Err(e) = store.add_document(&page.title, &page.text, 500, 50) {
                            log::warn!("索引失败 {}: {}", page.title, e);
                        }
                        added += 1;
                    }
                    Err(e) => {
                        log::warn!("爬取失败: {}", e);
                    }
                }
            }

            if added > 0 {
                let _ = tx.send(KbIndexMsg::Done {
                    status: format!("完成: {} 个文档已索引", added),
                    status_message: format!("已爬取 {} 个文档", added),
                    clear_url: true,
                    clear_title: false,
                    clear_content: false,
                });
            } else {
                let _ = tx.send(KbIndexMsg::Error(
                    "未爬取到有效内容。页面可能需 JavaScript 渲染，或 URL 不正确。".into(),
                ));
            }
        }
    }
}

/// 分块 + 向量化 + 入库，返回 `(成功段数, 总段数)`。
/// 大文档自动分段并逐段上报进度；单段文档直接入库。
fn index_content(
    store: &VectorStore,
    title: &str,
    content: &str,
    chunk_size: usize,
    overlap: usize,
    cancel: &AtomicBool,
    tx: &mpsc::Sender<KbIndexMsg>,
) -> Result<(usize, usize), String> {
    let char_count = content.chars().count();
    if char_count > crate::store::config::KB_SINGLE_DOC_CHARS {
        let chunks = crate::rag::chunker::chunk_text(content, chunk_size, overlap);
        let total = chunks.len();
        let mut added = 0usize;
        for (i, chunk) in chunks.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err("已取消".into());
            }
            let _ = tx.send(KbIndexMsg::Progress {
                frac: 0.3 + (i as f32 / total as f32) * 0.65,
                status: format!("索引分段 {}/{}", i + 1, total),
            });
            let seg_title = format!("{} 段{}", title, i + 1);
            match store.add_document(&seg_title, chunk, chunk_size, overlap) {
                Ok(()) => added += 1,
                Err(e) => log::warn!("索引段失败 {}: {}", seg_title, e),
            }
        }
        Ok((added, total))
    } else {
        store
            .add_document(title, content, chunk_size, overlap)
            .map_err(|e| format!("索引失败: {}", e))?;
        Ok((1, 1))
    }
}

/// 统一发送索引收尾消息：成功 → Done，失败 → Error。
fn finish_kb_job(
    tx: &mpsc::Sender<KbIndexMsg>,
    result: Result<(usize, usize), String>,
    info: KbDoneInfo,
) {
    match result {
        Ok((added, total)) => {
            if total > 1 {
                let _ = tx.send(KbIndexMsg::Done {
                    status: "完成".into(),
                    status_message: format!("文档较长，已自动切分为 {}/{} 段索引", added, total),
                    clear_url: info.clear_url,
                    clear_title: info.clear_title,
                    clear_content: info.clear_content,
                });
            } else {
                let _ = tx.send(KbIndexMsg::Done {
                    status: info.single_status,
                    status_message: info.single_message,
                    clear_url: info.clear_url,
                    clear_title: info.clear_title,
                    clear_content: info.clear_content,
                });
            }
        }
        Err(e) => {
            let _ = tx.send(KbIndexMsg::Error(e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_file_size_allows_file_within_cap() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("small.txt");
        std::fs::write(&p, "0123456789").unwrap(); // 10 字节
        assert!(ensure_file_size(&p, 10).is_ok());
        assert!(ensure_file_size(&p, 100).is_ok());
    }

    #[test]
    fn ensure_file_size_rejects_oversized_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("big.txt");
        std::fs::write(&p, vec![b'x'; 20]).unwrap(); // 20 字节
        let err = ensure_file_size(&p, 10).unwrap_err();
        assert!(err.contains("文件过大"), "错误信息应说明原因: {err}");
    }

    #[test]
    fn ensure_file_size_missing_file_is_err() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("nonexistent.txt");
        assert!(ensure_file_size(&p, 1024).is_err());
    }
}
