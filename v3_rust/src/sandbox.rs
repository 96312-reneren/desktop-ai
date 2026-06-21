use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_FILE_SIZE: u64 = 500_000;

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
    fn safe_path(&self, relative: &str) -> Result<PathBuf, String> {
        let cleaned = relative.replace('\\', "/")
            .trim_matches('/')
            .to_string();
        if cleaned.is_empty() || cleaned == "." {
            return Ok(self.root.clone());
        }
        let candidate = self.root.join(&cleaned);

        match std::fs::canonicalize(&candidate) {
            Ok(resolved) => {
                if resolved.starts_with(&self.resolved_root) {
                    Ok(resolved)
                } else {
                    Err(format!("路径越界: {}", relative))
                }
            }
            Err(_) => {
                // Path doesn't exist yet, check logically
                let root_canon = self.root.to_string_lossy().to_string().replace('\\', "/");
                let candidate_canon = candidate.to_string_lossy().to_string().replace('\\', "/");
                if candidate_canon.starts_with(&root_canon) {
                    Ok(candidate)
                } else {
                    Err(format!("路径越界: {}", relative))
                }
            }
        }
    }

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
    use std::path::PathBuf;

    fn test_sandbox() -> Sandbox {
        let dir = std::env::temp_dir().join("desktop_ai_sandbox_test");
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

    #[test]
    fn test_create_subdir() {
        let sb = test_sandbox();
        sb.write("sub/dir/file.txt", "nested").unwrap();
        assert_eq!(sb.read("sub/dir/file.txt").unwrap(), "nested");
        let entries = sb.list("sub").unwrap();
        assert_eq!(entries.len(), 1);
    }
}
