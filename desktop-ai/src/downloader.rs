use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use sha2::{Digest, Sha256};

/// 验证下载目标路径不超出模型目录范围，防止路径遍历攻击。
/// 检查文件名不包含 `..`、`/` 或 `\`，且解析后的绝对路径以 `models_dir` 为前缀。
pub fn validate_download_path(dest: &Path) -> Result<(), String> {
    // 检查路径中不包含 .. 组件（防止路径遍历攻击）。
    // Path::components() 将 .. 解析为 ParentDir，将正常名称解析为 Normal。
    // 必须遍历所有组件，而不能只检查 file_name()，因为 .. 可能出现在任意祖先目录中。
    for component in dest.components() {
        match component {
            std::path::Component::ParentDir => {
                return Err("路径包含 .. 遍历组件".into());
            }
            std::path::Component::Normal(c) => {
                if let Some(s) = c.to_str() {
                    if s.contains("..") {
                        return Err(format!("路径组件包含非法字符: {}", s));
                    }
                }
            }
            _ => {}
        }
    }

    if dest.file_name().is_none() {
        return Err("无法解析文件名".into());
    }

    // 解析绝对路径并确认在模型目录内
    let models_dir = crate::config::models_dir();
    let resolved_models = std::fs::canonicalize(&models_dir).unwrap_or_else(|_| models_dir.clone());

    // 如果目标已存在，用 canonicalize 解析；否则先创建父目录再解析
    let resolved_dest = if dest.exists() {
        std::fs::canonicalize(dest).map_err(|e| format!("解析目标路径失败: {}", e))?
    } else {
        let parent = dest.parent().ok_or("目标路径无父目录")?;
        // 先创建父目录，支持子目录布局（如 qwen/7b/model.gguf）
        std::fs::create_dir_all(parent).map_err(|e| format!("创建父目录失败: {}", e))?;
        let resolved_parent =
            std::fs::canonicalize(parent).map_err(|e| format!("解析父目录失败: {}", e))?;
        let fname = dest.file_name().ok_or("目标路径无文件名")?;
        resolved_parent.join(fname)
    };

    if !resolved_dest.starts_with(&resolved_models) {
        return Err(format!("下载目标路径超出模型目录范围: {:?}", dest));
    }

    Ok(())
}

/// 从 URL 中提取文件名，并验证其不包含路径遍历字符。
fn safe_filename_from_url(url: &str) -> Result<String, String> {
    let filename = url
        .rsplit('/')
        .next()
        .unwrap_or("unknown")
        .split('?')
        .next()
        .unwrap_or("unknown");

    if filename.is_empty()
        || filename.contains("..")
        || filename.contains('/')
        || filename.contains('\\')
    {
        return Err(format!("URL 中的文件名不合法: {}", filename));
    }

    Ok(filename.to_string())
}

#[derive(Debug)]
pub enum DownloadMsg {
    Progress {
        percent: u32,
        downloaded_mb: f64,
        total_mb: f64,
    },
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
    parts: &[crate::config::ModelPart],
) {
    if parts.is_empty() {
        // 单文件模型：原路径不变
        match download_single_file(url, &dest, &cancel, &tx, expected_sha256, None) {
            Ok(()) => {
                let _ = tx.send(DownloadMsg::Done);
            }
            Err(_) => { /* 错误消息已在函数内发出 */ }
        }
    } else {
        download_model_parts(parts, &dest, &cancel, &tx);
    }
}

/// 分卷模型下载：逐卷下载（每卷独立续传 + SHA-256 校验），全部就绪后
/// 按序拼接为完整 GGUF 文件并清理分卷。取消后已下载分卷保留，下次续传。
fn download_model_parts(
    parts: &[crate::config::ModelPart],
    dest: &Path,
    cancel: &Arc<AtomicBool>,
    tx: &mpsc::Sender<DownloadMsg>,
) {
    if let Err(e) = validate_download_path(dest) {
        let _ = tx.send(DownloadMsg::Error(format!("路径安全验证失败: {}", e)));
        return;
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).ok();
    }

    let part_paths: Vec<PathBuf> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| dest.with_file_name(format!("{}.part{}", p.filename, i + 1)))
        .collect();
    let total = parts.len();

    // 分卷文件名安全验证：防止含 .. 或路径分隔符的恶意文件名
    // 将 part 文件写出模型目录（路径遍历攻击）。
    for (part, path) in parts.iter().zip(&part_paths) {
        if part.filename.is_empty()
            || part.filename.contains("..")
            || part.filename.contains('/')
            || part.filename.contains('\\')
        {
            let _ = tx.send(DownloadMsg::Error(format!(
                "分卷文件名不合法: {}",
                part.filename
            )));
            return;
        }
        if let Err(e) = validate_download_path(path) {
            let _ = tx.send(DownloadMsg::Error(format!("分卷路径安全验证失败: {}", e)));
            return;
        }
    }

    for (i, (part, path)) in parts.iter().zip(&part_paths).enumerate() {
        if cancel.load(Ordering::Relaxed) {
            let _ = tx.send(DownloadMsg::Status("已取消（进度已保留）".into()));
            return;
        }
        let _ = tx.send(DownloadMsg::Status(format!(
            "下载分卷 {}/{} ...",
            i + 1,
            total
        )));
        if let Err(e) = download_single_file(
            &part.url,
            path,
            cancel,
            tx,
            part.sha256.as_deref(),
            Some((i, total)),
        ) {
            log::warn!("part {} download failed: {}", part.filename, e);
            return; // 错误消息已发出
        }
    }

    let _ = tx.send(DownloadMsg::Status("合并分卷...".into()));
    if let Err(e) = merge_parts(dest, &part_paths) {
        // 删除不完整的合并产物，保留分卷以便重试
        let _ = fs::remove_file(dest);
        let _ = tx.send(DownloadMsg::Error(format!("合并失败: {}", e)));
        return;
    }

    let _ = tx.send(DownloadMsg::Done);
}

/// 将分卷文件按序拼接为最终模型文件，成功后清理分卷文件。
fn merge_parts(dest: &Path, part_paths: &[PathBuf]) -> Result<(), String> {
    let mut out = File::create(dest).map_err(|e| format!("创建合并文件失败: {}", e))?;
    let mut buf = vec![0u8; 1024 * 1024];
    for p in part_paths {
        let mut f = File::open(p).map_err(|e| format!("打开分卷 {} 失败: {}", p.display(), e))?;
        loop {
            let n = f
                .read(&mut buf)
                .map_err(|e| format!("读取分卷 {} 失败: {}", p.display(), e))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])
                .map_err(|e| format!("写入合并文件失败: {}", e))?;
        }
    }
    drop(out);
    for p in part_paths {
        if let Err(e) = fs::remove_file(p) {
            // 分卷残留不影响使用，仅告警
            log::warn!("failed to remove part file {}: {}", p.display(), e);
        }
    }
    Ok(())
}

/// 单文件下载核心：断点续传、取消、SHA-256 校验。
/// `part_index = Some((i, n))` 表示该文件是 n 个分卷中的第 i 个（0 起），
/// 进度百分比折算到整体范围，便于 UI 展示总进度。
fn download_single_file(
    url: &str,
    dest: &Path,
    cancel: &Arc<AtomicBool>,
    tx: &mpsc::Sender<DownloadMsg>,
    expected_sha256: Option<&str>,
    part_index: Option<(usize, usize)>,
) -> Result<(), String> {
    let _ = tx.send(DownloadMsg::Status("正在连接...".into()));

    // 路径安全验证：确保目标在模型目录内，URL 文件名不含路径遍历字符
    if let Err(e) = validate_download_path(dest) {
        let _ = tx.send(DownloadMsg::Error(format!("路径安全验证失败: {}", e)));
        return Err(e);
    }
    if let Err(e) = safe_filename_from_url(url) {
        let _ = tx.send(DownloadMsg::Error(format!("URL 安全验证失败: {}", e)));
        return Err(e);
    }

    // Check existing file for resume
    let existing_size = if dest.exists() {
        fs::metadata(dest).map(|m| m.len()).unwrap_or(0)
    } else {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).ok();
        }
        0
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .danger_accept_invalid_certs(false)
        .danger_accept_invalid_hostnames(false)
        .build();

    let client = match client {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(DownloadMsg::Error(format!("创建连接失败: {}", e)));
            return Err(e.to_string());
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
            return Err(e.to_string());
        }
    };

    let status = response.status();
    let (total_size, mut downloaded, mut file) = if status == 206 {
        let total = response
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split('/').next_back()?.parse().ok())
            .unwrap_or(0);

        // If the server's total is smaller than what we already have, the
        // local file is corrupt — restart from scratch.
        if total > 0 && existing_size > total {
            let _ = tx.send(DownloadMsg::Status(format!(
                "本地文件不完整 ({:.0} MB > {:.0} MB)，重新下载...",
                existing_size as f64 / 1_048_576.0,
                total as f64 / 1_048_576.0
            )));
            let _ = fs::remove_file(dest);
            let req = client.get(url);
            match req.send() {
                Ok(r) if r.status() == 200 => {
                    let total = r
                        .headers()
                        .get("content-length")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let f = File::create(dest);
                    match f {
                        Ok(f) => (total, 0u64, f),
                        Err(e) => {
                            let _ = tx.send(DownloadMsg::Error(format!("无法创建文件: {}", e)));
                            return Err(e.to_string());
                        }
                    }
                }
                Ok(r) => {
                    let _ = tx.send(DownloadMsg::Error(format!("HTTP {}", r.status())));
                    return Err(format!("HTTP {}", r.status()));
                }
                Err(e) => {
                    let _ = tx.send(DownloadMsg::Error(format!("连接失败: {}", e)));
                    return Err(e.to_string());
                }
            }
        } else {
            let _ = tx.send(DownloadMsg::Status(format!(
                "续传中 ({:.0}/{:.0} MB)...",
                existing_size as f64 / 1_048_576.0,
                total as f64 / 1_048_576.0
            )));

            let f = OpenOptions::new().append(true).open(dest);
            match f {
                Ok(f) => (total, existing_size, f),
                Err(e) => {
                    let _ = tx.send(DownloadMsg::Error(format!("无法写入文件: {}", e)));
                    return Err(e.to_string());
                }
            }
        }
    } else if status == 200 {
        let total = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let f = File::create(dest);
        match f {
            Ok(f) => (total, 0u64, f),
            Err(e) => {
                let _ = tx.send(DownloadMsg::Error(format!("无法创建文件: {}", e)));
                return Err(e.to_string());
            }
        }
    } else if status == 416 {
        // Range not satisfiable — the local file is already complete.
        // Fall through to the integrity check below.
        let _ = tx.send(DownloadMsg::Status("文件已完整，正在校验...".into()));
        match File::open(dest) {
            Ok(f) => (existing_size, existing_size, f),
            Err(e) => {
                let _ = tx.send(DownloadMsg::Error(format!("无法打开文件: {}", e)));
                return Err(e.to_string());
            }
        }
    } else {
        let _ = tx.send(DownloadMsg::Error(format!("HTTP {}", status)));
        return Err(format!("HTTP {}", status));
    };

    let mut reader = response;
    let mut buf = vec![0u8; 1024 * 1024]; // 1MB buffer

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = tx.send(DownloadMsg::Status("已取消（进度已保留）".into()));
            return Err("cancelled".into());
        }

        let bytes = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                let _ = tx.send(DownloadMsg::Error(format!("下载中断: {}", e)));
                return Err(e.to_string());
            }
        };

        if let Err(e) = file.write_all(&buf[..bytes]) {
            let _ = tx.send(DownloadMsg::Error(format!("写入失败: {}", e)));
            return Err(e.to_string());
        }

        downloaded += bytes as u64;

        if total_size > 0 {
            let part_pct = downloaded as f64 / total_size as f64;
            // 分卷场景折算到整体进度；单文件场景直接使用当前百分比
            let pct = match part_index {
                Some((i, n)) => ((i as f64 + part_pct) / n as f64 * 100.0) as u32,
                None => (part_pct * 100.0) as u32,
            };
            let _ = tx.send(DownloadMsg::Progress {
                percent: pct,
                downloaded_mb: downloaded as f64 / 1_048_576.0,
                total_mb: total_size as f64 / 1_048_576.0,
            });
        }
    }

    drop(file);

    let actual_size = fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    if actual_size < 50_000_000 {
        if let Err(e) = fs::remove_file(dest) {
            log::warn!("failed to remove corrupted download: {}", e);
        }
        let _ = tx.send(DownloadMsg::Error(
            "下载文件异常小，已删除。请检查网络后重试。".into(),
        ));
        return Err("file too small".into());
    }

    // Integrity check: if the model catalog supplies an expected SHA-256,
    // verify it before signalling success. Mismatch deletes the corrupt file.
    if let Some(expected) = expected_sha256 {
        let _ = tx.send(DownloadMsg::Status("校验完整性 (SHA-256)...".into()));
        match compute_sha256(dest) {
            Ok(actual) => {
                if !actual.eq_ignore_ascii_case(expected.trim()) {
                    let _ = fs::remove_file(dest);
                    let _ = tx.send(DownloadMsg::Error(format!(
                        "SHA-256 校验失败\n期望: {}\n实际: {}",
                        expected, actual
                    )));
                    return Err("sha256 mismatch".into());
                }
            }
            Err(e) => {
                let _ = tx.send(DownloadMsg::Error(format!("校验失败: {}", e)));
                return Err(e);
            }
        }
    }

    Ok(())
}

fn compute_sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("打开文件失败: {}", e))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("读取失败: {}", e))?;
        if n == 0 {
            break;
        }
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
        assert_eq!(
            h,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
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
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_download_path_safe() {
        // models_dir() 下的正常文件名应通过验证
        let models = crate::config::models_dir();
        let dest = models.join("model.gguf");
        assert!(validate_download_path(&dest).is_ok());
    }

    #[test]
    fn test_validate_download_path_traversal_in_filename() {
        // 文件名包含 ".." 应被拒绝
        let models = crate::config::models_dir();
        let dest = models.join("../evil.gguf");
        assert!(validate_download_path(&dest).is_err());
    }

    #[test]
    fn test_validate_download_path_outside_models() {
        // 绝对路径在模型目录外应被拒绝
        let dest = std::env::temp_dir().join("outside.gguf");
        assert!(validate_download_path(&dest).is_err());
    }

    #[test]
    fn test_validate_download_path_subdirectory() {
        // 子目录路径（父目录尚未创建）应能通过验证
        let models = crate::config::models_dir();
        let dest = models.join("qwen").join("7b").join("model.gguf");
        assert!(validate_download_path(&dest).is_ok());
        // 清理测试创建的目录
        let _ = fs::remove_dir_all(models.join("qwen"));
    }

    #[test]
    fn test_validate_download_path_rejects_traversal_in_subdir() {
        // 子目录路径中包含 .. 应被拒绝（starts_with 前缀校验拦截）
        let models = crate::config::models_dir();
        let dest = models.join("..").join("evil.gguf");
        assert!(validate_download_path(&dest).is_err());
    }

    #[test]
    fn test_validate_download_path_rejects_direct_traversal() {
        // 直接 ../ 路径遍历应被拒绝
        let models = crate::config::models_dir();
        let dest = models.join("../evil.gguf");
        assert!(validate_download_path(&dest).is_err());
    }

    #[test]
    fn test_safe_filename_from_url_normal() {
        let name = safe_filename_from_url("https://example.com/models/llama-3.gguf").unwrap();
        assert_eq!(name, "llama-3.gguf");
    }

    #[test]
    fn test_safe_filename_from_url_with_query() {
        let name = safe_filename_from_url("https://example.com/model.gguf?token=abc").unwrap();
        assert_eq!(name, "model.gguf");
    }

    #[test]
    fn test_safe_filename_from_url_traversal_rejected() {
        // URL 末尾段包含 ".." 应被拒绝
        assert!(safe_filename_from_url("https://example.com/models/..").is_err());
    }

    #[test]
    fn test_safe_filename_from_url_empty_rejected() {
        assert!(safe_filename_from_url("https://example.com/").is_err());
    }

    #[test]
    fn test_merge_parts_appends_in_order() {
        // 两个分卷按序拼接，且 part 文件被清理
        let dir = std::env::temp_dir().join("desktop_ai_merge_test");
        let _ = fs::create_dir_all(&dir);
        let dest = dir.join("model.gguf");
        let p1 = dir.join("model.part1");
        let p2 = dir.join("model.part2");
        fs::write(&p1, b"hello ").unwrap();
        fs::write(&p2, b"world").unwrap();

        merge_parts(&dest, &[p1.clone(), p2.clone()]).unwrap();
        let merged = fs::read(&dest).unwrap();
        assert_eq!(merged, b"hello world");
        assert!(!p1.exists() && !p2.exists(), "part 文件应被清理");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_merge_parts_missing_part_fails() {
        // 缺少分卷时合并应失败且不产生半成品
        let dir = std::env::temp_dir().join("desktop_ai_merge_test2");
        let _ = fs::create_dir_all(&dir);
        let dest = dir.join("model.gguf");
        let p1 = dir.join("model.part1");
        let p2 = dir.join("model.part2");
        fs::write(&p1, b"hello ").unwrap();
        // p2 故意不创建
        assert!(merge_parts(&dest, &[p1.clone(), p2.clone()]).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_part_filename_traversal_rejected() {
        // 分卷文件名含 .. 时 download_model_parts 应拒绝并报错
        let models = crate::config::models_dir();
        let dir = models.join("part_traversal_test");
        let _ = fs::create_dir_all(&dir);
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let evil = crate::config::ModelPart {
            url: "https://example.com/x.gguf".into(),
            filename: "../evil.gguf".into(),
            sha256: None,
        };
        download_model_parts(&[evil], &dir.join("model.gguf"), &cancel, &tx);
        let mut msgs = Vec::new();
        while let Ok(m) = rx.try_recv() {
            msgs.push(m);
        }
        assert!(
            msgs.iter().any(|m| matches!(m, DownloadMsg::Error(_))),
            "恶意分卷文件名应报错，实际: {:?}",
            msgs
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
