#![no_main]
//! fuzz `chunk_text`：句感知分块。锁定两条性质：
//! 1. 任意输入（含极端 overlap、超长单句）不 panic、不死循环；
//! 2. 每个输出块的字符数严格 ≤ chunk_size（chunk_size ≥ 1）。
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    text: String,
    chunk_size: u8,
    overlap: u8,
}

fuzz_target!(|input: Input| {
    let chunk_size = input.chunk_size.max(1) as usize;
    let overlap = input.overlap as usize;
    let chunks = desktop_ai::chunk_text(&input.text, chunk_size, overlap);
    for c in &chunks {
        assert!(
            c.chars().count() <= chunk_size,
            "chunk ({}) exceeds chunk_size ({})",
            c.chars().count(),
            chunk_size
        );
    }
});
