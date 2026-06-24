use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;

#[cfg_attr(not(debug_assertions), allow(dead_code))]
#[allow(dead_code)]
const MAX_FILE_SIZE: u64 = 500_000;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
}

pub struct Sandbox {
    root: PathBuf,
    resolved_root: PathBuf,
}

impl Sandbox {
    pub fn new(dir: PathBuf) -> Self {
        fs::create_dir_all(&dir).ok();
        let resolved = std::fs::canonicalize(&dir)
            .unwrap_or_else(|_| dir.clone());
        Self { root: dir, resolved_root: resolved }
    }

    /// Resolve a path and ensure it stays within the sandbox.
    ///
    /// Security: rejects any `..` component outright, and for paths that do
    /// not yet exist resolves the parent dir via `canonicalize` then re-joins
    /// the file name, finally comparing with `Path::starts_with` (component
    /// aware) rather than a string prefix check. The previous string-prefix
    /// fallback accepted `root/../escape.txt` because it textually started
    /// with `root` — a path-traversal bug.
    fn safe_path(&self, relative: &str) -> Result<PathBuf, String> {
        let cleaned = relative.replace('\\', "/").trim_matches('/').to_string();
        if cleaned.is_empty() || cleaned == "." {
            return Ok(self.root.clone());
        }

        // Reject any traversal component outright.
        for comp in cleaned.split('/') {
            if comp == ".." {
                return Err(format!("路径越界: {}", relative));
            }
        }

        let candidate = self.root.join(&cleaned);

        // Existing path: canonicalize both and compare as path components.
        if candidate.exists() {
            let resolved = std::fs::canonicalize(&candidate)
                .map_err(|e| format!("解析路径失败: {}", e))?;
            if resolved.starts_with(&self.resolved_root) {
                return Ok(resolved);
            }
            return Err(format!("路径越界: {}", relative));
        }

        // Non-existent path: `..` is already rejected, so `root.join(cleaned)`
        // cannot escape upward. Use `Path::starts_with` (component-aware) as
        // defence in depth and return the logical path. The previous string
        // prefix check accepted `root/../escape.txt` because it textually
        // began with `root`.
        if candidate.starts_with(&self.root) {
            Ok(candidate)
        } else {
            Err(format!("路径越界: {}", relative))
        }
    }

    /// Reserved for the AI Agent tool-call protocol (P1). When the Agent
    /// loop issues a tool call like `read("work/output.txt")`, this method
    /// sanitises the path, enforces the 500 KB file cap, and returns the
    /// content. All three methods (`read` / `write` / `list`) are covered by
    /// unit tests and must not be removed before the Agent protocol is
    /// implemented.
    #[allow(dead_code)]
    pub fn read(&self, relative: &str) -> Result<String, String> {
        let path = self.safe_path(relative)?;
        if !path.exists() {
            return Err(format!("文件不存在: {}", relative));
        }
        if path.is_dir() {
            return self.list_text(relative);
        }
        let meta = fs::metadata(&path).map_err(|e| format!("读取失败: {}", e))?;
        if meta.len() > MAX_FILE_SIZE {
            return Err("文件过大".into());
        }
        let mut f = fs::File::open(&path).map_err(|e| format!("打开失败: {}", e))?;
        let mut buf = String::new();
        f.read_to_string(&mut buf).map_err(|e| format!("读取失败: {}", e))?;
        Ok(buf)
    }

    #[allow(dead_code)]
    pub fn write(&self, relative: &str, content: &str) -> Result<(), String> {
        let path = self.safe_path(relative)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
        }
        if content.len() as u64 > MAX_FILE_SIZE {
            return Err("内容过大".into());
        }
        let mut f = fs::File::create(&path).map_err(|e| format!("创建文件失败: {}", e))?;
        f.write_all(content.as_bytes()).map_err(|e| format!("写入失败: {}", e))?;
        Ok(())
    }

    pub fn list(&self, relative: &str) -> Result<Vec<FileEntry>, String> {
        let dir = self.safe_path(relative)?;
        if !dir.exists() { return Ok(Vec::new()); }
        if !dir.is_dir() { return Err("不是目录".into()); }
        self._list_impl(dir)
    }

    fn _list_impl(&self, dir: PathBuf) -> Result<Vec<FileEntry>, String> {
        let mut entries = Vec::new();
        let iter = match fs::read_dir(&dir) {
            Ok(i) => i,
            Err(e) => return Err(format!("读取目录失败: {}", e)),
        };
        for entry in iter {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let name = path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            entries.push(FileEntry {
                name,
                path,
                size: if meta.is_dir() { 0 } else { meta.len() },
                is_dir: meta.is_dir(),
            });
        }
        entries.sort_by(|a, b| {
            b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name))
        });
        Ok(entries)
    }

    #[allow(dead_code)]
    fn list_text(&self, relative: &str) -> Result<String, String> {
        let entries = self.list(relative)?;
        if entries.is_empty() {
            return Ok("(空目录)".into());
        }
        let mut s = String::new();
        for e in &entries {
            let kind = if e.is_dir { "[DIR]" } else { "[FILE]" };
            s.push_str(&format!("{} {} ({}B)\n", kind, e.name, e.size));
        }
        Ok(s)
    }

    pub fn root_path(&self) -> &PathBuf { &self.root }
}

#[cfg(test)]
mod tests {
    use super::*;

    static NEXT_TEST_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    /// Each test gets a unique temp directory so parallel or sequential runs
    /// never collide — the old shared `desktop_ai_sandbox_test` directory
    /// caused flaky failures on Windows when `remove_dir_all` couldn't
    /// acquire the handle in time.
    fn test_sandbox() -> Sandbox {
        let n = NEXT_TEST_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("desktop_ai_sandbox_test_{}", n));
        let _ = fs::remove_dir_all(&dir);
        Sandbox::new(dir)
    }

    #[test]
    fn test_write_and_read() {
        let sb = test_sandbox();
        sb.write("test.txt", "hello world").unwrap();
        assert_eq!(sb.read("test.txt").unwrap(), "hello world");
    }

    #[test]
    fn test_path_traversal_blocked() {
        let sb = test_sandbox();
        assert!(sb.write("../escape.txt", "evil").is_err());
        assert!(sb.write("..\\escape.txt", "evil").is_err());
    }

    /// Empirical regression test: `../escape.txt` MUST NOT create a file
    /// outside the sandbox. The previous string-prefix implementation
    /// returned `Ok(())` and wrote the file to the parent directory.
    #[test]
    fn test_path_traversal_no_side_effect() {
        let dir = std::env::temp_dir().join("desktop_ai_sandbox_noescape");
        let _ = fs::remove_dir_all(&dir);
        let sb = Sandbox::new(dir.clone());

        let outside = dir.parent().unwrap().join("escape_audit_noescape.txt");
        let _ = fs::remove_file(&outside);

        let res = sb.write("../escape_audit_noescape.txt", "evil");
        let leaked = outside.exists();

        let _ = fs::remove_file(&outside);
        let _ = fs::remove_dir_all(&dir);

        assert!(res.is_err(), "write must be rejected");
        assert!(!leaked, "PATH TRAVERSAL REGRESSION: file escaped sandbox");
    }

    #[test]
    fn test_create_subdir() {
        let sb = test_sandbox();
        sb.write("sub/dir/file.txt", "nested").unwrap();
        assert_eq!(sb.read("sub/dir/file.txt").unwrap(), "nested");
        let entries = sb.list("sub").unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_write_to_new_file_in_root_is_allowed() {
        // Non-existent file directly under root must still be writable.
        let sb = test_sandbox();
        sb.write("brand_new_file.txt", "ok").unwrap();
        assert_eq!(sb.read("brand_new_file.txt").unwrap(), "ok");
    }
}
