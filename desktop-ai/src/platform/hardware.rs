//! Hardware detection (CPU cores, RAM, GPUs).
//!
//! Moved out of the UI layer: this is platform integration, not rendering.
//! Used by the model picker to warn about undersized machines.

#[derive(Clone, Debug)]
pub(crate) struct GpuInfo {
    pub name: String,
    pub vram_gb: f64,
}

pub(crate) fn detect_hardware() -> (usize, Option<String>) {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);

    let ram_gb = get_total_ram_gb();
    let warning = if ram_gb > 0.0 && ram_gb < 4.0 {
        Some(format!(
            "⚠ 检测到内存仅 {:.1} GB，建议只使用 0.5B 或 1.7B 模型。大模型会严重卡顿或无法加载。",
            ram_gb
        ))
    } else if ram_gb > 0.0 && ram_gb < 8.0 {
        Some(format!(
            "ℹ 检测到内存 {:.1} GB，可使用 3B 以下的模型。7B+ 模型需要 8GB 以上内存。",
            ram_gb
        ))
    } else {
        None
    };
    (cores, warning)
}

#[cfg(windows)]
pub(crate) fn get_total_ram_gb() -> f64 {
    use std::mem;
    unsafe {
        let mut mem_status: windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX =
            mem::zeroed();
        mem_status.dwLength =
            mem::size_of::<windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX>() as u32;
        if windows_sys::Win32::System::SystemInformation::GlobalMemoryStatusEx(&mut mem_status) != 0
        {
            mem_status.ullTotalPhys as f64 / (1024.0 * 1024.0 * 1024.0)
        } else {
            0.0
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn get_total_ram_gb() -> f64 {
    0.0
}

#[cfg(windows)]
pub(crate) fn detect_gpus() -> Vec<GpuInfo> {
    let output = std::process::Command::new("wmic")
        .args([
            "path",
            "Win32_VideoController",
            "get",
            "Name,AdapterRAM",
            "/format:csv",
        ])
        .output();
    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut gpus = Vec::new();
            for line in text.lines().skip(2) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() >= 3 {
                    let name = parts[1].trim().to_string();
                    let ram_bytes: u64 = parts[2].trim().parse().unwrap_or(0);
                    let vram = ram_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                    if vram > 0.0 && !name.is_empty() && !name.contains("Microsoft Basic") {
                        gpus.push(GpuInfo {
                            name,
                            vram_gb: vram,
                        });
                    }
                }
            }
            gpus
        }
        Err(_) => Vec::new(),
    }
}

#[cfg(not(windows))]
pub(crate) fn detect_gpus() -> Vec<GpuInfo> {
    Vec::new()
}
