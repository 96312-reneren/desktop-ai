//! Desktop shortcut creation (first-run prompt).
//!
//! Windows: writes a `.lnk` via the WScript.Shell COM object (no extra
//! dependency). Linux: writes a `.desktop` launcher into the user's
//! Desktop and the applications menu. The target path is always the
//! *current* executable so portable copies on USB sticks work too.

use std::path::PathBuf;

/// True when a shortcut for the current executable already exists.
pub fn shortcut_exists() -> bool {
    shortcut_path().map(|p| p.exists()).unwrap_or(false)
}

/// Path where the shortcut would live (None when undeterminable).
fn shortcut_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_name = exe.file_name()?.to_string_lossy().to_string();
    #[cfg(target_os = "windows")]
    {
        let home = std::env::var("USERPROFILE").ok()?;
        Some(
            PathBuf::from(home)
                .join("Desktop")
                .join(format!("{}.lnk", exe_name.trim_end_matches(".exe"))),
        )
    }
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").ok()?;
        Some(
            PathBuf::from(home)
                .join("Desktop")
                .join("desktop-ai.desktop"),
        )
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = exe_name;
        None
    }
}

/// Create a desktop shortcut for the running executable.
/// Returns `Ok(true)` on success, `Ok(false)` when skipped (no desktop),
/// `Err` with a message on failure.
pub fn create_desktop_shortcut() -> Result<bool, String> {
    let exe = std::env::current_exe().map_err(|e| format!("获取程序路径失败: {}", e))?;
    let exe_dir = exe
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "无法定位程序目录".to_string())?;

    #[cfg(target_os = "windows")]
    {
        let link_path = shortcut_path().ok_or_else(|| "无法定位桌面目录".to_string())?;
        if link_path.exists() {
            return Ok(true);
        }
        // PowerShell + WScript.Shell: create a .lnk pointing at the exe.
        let script = format!(
            "$ws = New-Object -ComObject WScript.Shell; \
             $s = $ws.CreateShortcut('{}'); \
             $s.TargetPath = '{}'; \
             $s.WorkingDirectory = '{}'; \
             $s.Save()",
            link_path.display(),
            exe.display(),
            exe_dir.display()
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .map_err(|e| format!("启动 PowerShell 失败: {}", e))?;
        if out.status.success() {
            Ok(true)
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").map_err(|_| "无法定位 HOME".to_string())?;
        let desktop_entry = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=桌面AI\n\
             Comment=本地大模型聊天应用\n\
             Exec={}\n\
             Path={}\n\
             Terminal=false\n\
             Categories=Utility;\n",
            exe.display(),
            exe_dir.display()
        );
        // Desktop icon (when a Desktop dir exists).
        let desktop_dir = PathBuf::from(&home).join("Desktop");
        let mut created = false;
        if desktop_dir.is_dir() {
            let target = desktop_dir.join("desktop-ai.desktop");
            std::fs::write(&target, &desktop_entry)
                .map_err(|e| format!("写入桌面快捷方式失败: {}", e))?;
            let _ = std::process::Command::new("chmod")
                .args(["u+x", &target.to_string_lossy()])
                .status();
            created = true;
        }
        // Applications menu entry (GNOME/KDE).
        let apps_dir = PathBuf::from(&home).join(".local/share/applications");
        let _ = std::fs::create_dir_all(&apps_dir);
        let target = apps_dir.join("desktop-ai.desktop");
        if let Err(e) = std::fs::write(&target, &desktop_entry) {
            if !created {
                return Err(format!("写入应用菜单项失败: {}", e));
            }
        } else {
            let _ = std::process::Command::new("chmod")
                .args(["u+x", &target.to_string_lossy()])
                .status();
            created = true;
        }
        if created {
            Ok(true)
        } else {
            Err("未找到桌面或应用菜单目录".into())
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = (exe, exe_dir);
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_path_is_never_absolute_mess() {
        // Just ensure the function does not panic and returns a plausible
        // path or None on unsupported platforms.
        if let Some(p) = shortcut_path() {
            assert!(p.to_string_lossy().len() > 4);
        }
    }
}
