use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

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
            .and_then(|s| s.split('/').last()?.parse().ok())
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

    let _ = tx.send(DownloadMsg::Done);
}
