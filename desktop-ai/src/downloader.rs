use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use sha2::{Digest, Sha256};

pub enum DownloadMsg {
    Progress { percent: u32, downloaded_mb: f64, total_mb: f64 },
    Status(String),
    Done,
    Error(String),
}

pub fn download_model(
    url: &str,
    dest: PathBuf,
    cancel: Arc<AtomicBool>,
    tx: mpsc::Sender<DownloadMsg>,
    expected_sha256: Option<&str>,
) {
    let _ = tx.send(DownloadMsg::Status("正在连接...".into()));

    // Check existing file for resume
    let existing_size = if dest.exists() {
        fs::metadata(&dest).map(|m| m.len()).unwrap_or(0)
    } else {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).ok();
        }
        0
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build();

    let client = match client {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(DownloadMsg::Error(format!("创建连接失败: {}", e)));
            return;
        }
    };

    let mut req = client.get(url);
    if existing_size > 0 {
        req = req.header("Range", format!("bytes={}-", existing_size));
    }

    let response = match req.send() {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(DownloadMsg::Error(format!("连接失败: {}", e)));
            return;
        }
    };

    let status = response.status();
    let (total_size, mut downloaded, mut file) = if status == 206 {
        let total = response.headers().get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split('/').next_back()?.parse().ok())
            .unwrap_or(0);

        let _ = tx.send(DownloadMsg::Status(format!(
            "续传中 ({:.0}/{:.0} MB)...", existing_size as f64/1e6, total as f64/1e6
        )));

        let f = OpenOptions::new().append(true).open(&dest);
        match f {
            Ok(f) => (total, existing_size, f),
            Err(e) => {
                let _ = tx.send(DownloadMsg::Error(format!("无法写入文件: {}", e)));
                return;
            }
        }
    } else if status == 200 {
        let total = response.headers().get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let f = File::create(&dest);
        match f {
            Ok(f) => (total, 0u64, f),
            Err(e) => {
                let _ = tx.send(DownloadMsg::Error(format!("无法创建文件: {}", e)));
                return;
            }
        }
    } else {
        let _ = tx.send(DownloadMsg::Error(format!("HTTP {}", status)));
        return;
    };

    let mut reader = response;
    let mut buf = vec![0u8; 1024 * 1024]; // 1MB buffer

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = tx.send(DownloadMsg::Status("已取消（进度已保留）".into()));
            return;
        }

        let bytes = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                let _ = tx.send(DownloadMsg::Error(format!("下载中断: {}", e)));
                return;
            }
        };

        if let Err(e) = file.write_all(&buf[..bytes]) {
            let _ = tx.send(DownloadMsg::Error(format!("写入失败: {}", e)));
            return;
        }

        downloaded += bytes as u64;

        if total_size > 0 {
            let pct = ((downloaded as f64 / total_size as f64) * 100.0) as u32;
            let _ = tx.send(DownloadMsg::Progress {
                percent: pct,
                downloaded_mb: downloaded as f64 / 1_048_576.0,
                total_mb: total_size as f64 / 1_048_576.0,
            });
        }
    }

    drop(file);

    let actual_size = fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    if actual_size < 50_000_000 {
        if let Err(e) = fs::remove_file(&dest) {
            log::warn!("failed to remove corrupted download: {}", e);
        }
        let _ = tx.send(DownloadMsg::Error("下载文件异常小，已删除。请检查网络后重试。".into()));
        return;
    }

    // Integrity check: if the model catalog supplies an expected SHA-256,
    // verify it before signalling success. Mismatch deletes the corrupt file.
    if let Some(expected) = expected_sha256 {
        let _ = tx.send(DownloadMsg::Status("校验完整性 (SHA-256)...".into()));
        match compute_sha256(&dest) {
            Ok(actual) => {
                if !actual.eq_ignore_ascii_case(expected.trim()) {
                    let _ = fs::remove_file(&dest);
                    let _ = tx.send(DownloadMsg::Error(format!(
                        "SHA-256 校验失败\n期望: {}\n实际: {}", expected, actual
                    )));
                    return;
                }
            }
            Err(e) => {
                let _ = tx.send(DownloadMsg::Error(format!("校验失败: {}", e)));
                return;
            }
        }
    }

    let _ = tx.send(DownloadMsg::Done);
}

fn compute_sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("打开文件失败: {}", e))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("读取失败: {}", e))?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    let hash = hasher.finalize();
    Ok(hash.iter().map(|b| format!("{:02x}", b)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_sha256_known_value() {
        // SHA-256 of empty string: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let dir = std::env::temp_dir().join("desktop_ai_sha_test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("empty.bin");
        fs::write(&path, b"").unwrap();
        let h = compute_sha256(&path).unwrap();
        assert_eq!(h, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_compute_sha256_abc() {
        // SHA-256("abc")
        let dir = std::env::temp_dir().join("desktop_ai_sha_test2");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("abc.bin");
        fs::write(&path, b"abc").unwrap();
        let h = compute_sha256(&path).unwrap();
        assert_eq!(h, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = fs::remove_dir_all(&dir);
    }
}
