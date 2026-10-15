#![no_main]
//! fuzz `parse_http_validated`：本机 API 服务器手写 HTTP 请求解析器。
//! 覆盖请求行 / 方法白名单 / URI 长度 / HTTP 版本 / 头部数量与尺寸 /
//! 控制字符 / 头名合法性 / 重复 Content-Length 等全部拒绝路径。
//! 锁定：任意字节流不 panic。
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let raw = String::from_utf8_lossy(data);
    let _ = desktop_ai::fuzz_api::parse_http_validated(&raw);
});
