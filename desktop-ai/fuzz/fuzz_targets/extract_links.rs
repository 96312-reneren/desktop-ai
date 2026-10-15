#![no_main]
//! fuzz `extract_links`：敌意 HTML 的 href 提取（未闭合引号、属性值
//! 穿插多字节字符、`javascript:` 过滤）。锁定：不 panic 且链接数 ≤ 50。
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    html: String,
    base_url: String,
}

fuzz_target!(|input: Input| {
    let links = desktop_ai::fuzz_api::extract_links(&input.html, &input.base_url);
    assert!(links.len() <= 50, "link cap violated: {}", links.len());
});
