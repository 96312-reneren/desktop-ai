#![no_main]
//! fuzz `clean_text`：HTML / Markdown / 纯文本三通道清洗管线。
//! 标题提取曾因 Unicode 大小写展开改变字节偏移而越界 panic，
//! 本 target 锁定任意字节流全管线不 panic。
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    raw: String,
    format_selector: u8,
}

fuzz_target!(|input: Input| {
    let format = match input.format_selector % 5 {
        0 => "html",
        1 => "markdown",
        2 => "md",
        3 => "text",
        _ => " TotallyUnknownFormat ", // 命中 _ 兜底分支
    };
    let (_title, _body) = desktop_ai::clean_text(&input.raw, format);
});
