//! Hardware detection (CPU cores, RAM, GPUs).
//!
//! Moved out of the UI layer: this is platform integration, not rendering.
//! Used by the model picker to warn about undersized machines.

/// 单块 GPU 的型号与显存信息，供模型选择器评估可承载的模型规模。
#[derive(Clone, Debug)]
pub(crate) struct GpuInfo {
    /// GPU 型号名称（来自系统枚举）。
    pub name: String,
    /// 显存容量，单位 GB。
    pub vram_gb: f64,
}

/// 探测本机逻辑 CPU 核数与物理内存，并在内存偏小时给出面向用户的模型选型警告。
///
/// 返回 `(核数, 可选警告)`：核数探测失败时回退为 2；内存探测失败（`0.0`）时不产生
/// 警告，避免把"未知"误报为"内存不足"。
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

/// 通过 Win32 `GlobalMemoryStatusEx` 读取物理内存总量，单位 GB。
///
/// 调用失败时返回 `0.0`，调用方需按"未知"语义处理，不得视为 0GB。
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

/// 非 Windows 平台的内存探测占位实现，恒返回 `0.0`（表示未知）。
#[cfg(not(windows))]
pub(crate) fn get_total_ram_gb() -> f64 {
    0.0
}

/// 通过 `wmic` 查询 `Win32_VideoController` 枚举 GPU 名称与显存（单位 GB）。
///
/// 过滤掉显存为 0 及 Microsoft Basic 虚拟显示适配器；`wmic` 不可用或解析失败时返回
/// 空列表，因此调用方必须容忍"检测不到 GPU"的情况。
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

/// 非 Windows 平台的 GPU 探测占位实现，恒返回空列表。
#[cfg(not(windows))]
pub(crate) fn detect_gpus() -> Vec<GpuInfo> {
    Vec::new()
}
