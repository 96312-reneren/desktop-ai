//! Windows 控制台附加：让无窗口的 `--serve` 模式也能在终端里输出。
//!
//! GUI 子系统（`windows_subsystem = "windows"`）的进程默认没有标准输出，
//! 直接在命令行运行时什么都看不到。这里在 Windows 上尽力挂到父进程的
//! 控制台，并把 stdout / stderr 重新绑定到 `CONOUT$`；失败时静默放弃
//! （例如从资源管理器双击启动，本来就没有父控制台），此时信息仍会写入
//! 数据目录下的 `serve.info` 与日志文件。
//!
//! 非 Windows 平台为空实现：终端本来就是普通控制台。

#[cfg(windows)]
pub(crate) fn attach_parent_console() {
    use std::ffi::c_void;

    // 附到调用方进程的控制台（`ATTACH_PARENT_PROCESS`）。
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
        fn SetStdHandle(std_handle: u32, handle: *mut c_void) -> i32;
        fn CreateFileW(
            file_name: *const u16,
            desired_access: u32,
            share_mode: u32,
            security_attributes: *mut c_void,
            creation_disposition: u32,
            flags_and_attributes: u32,
            template_file: *mut c_void,
        ) -> *mut c_void;
    }

    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4; // -12
    const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5; // -11
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const OPEN_EXISTING: u32 = 3;
    const INVALID_HANDLE_VALUE: *mut c_void = -1isize as *mut c_void;

    // SAFETY: 只调用三个 Win32 API；参数均为本函数内构造的合法值，
    // 宽字符串以 NUL 结尾，句柄在使用前检查有效性。失败分支不做任何操作。
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        let conout: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        let handle = CreateFileW(
            conout.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return;
        }
        SetStdHandle(STD_OUTPUT_HANDLE, handle);
        SetStdHandle(STD_ERROR_HANDLE, handle);
    }
}

#[cfg(not(windows))]
pub(crate) fn attach_parent_console() {}
