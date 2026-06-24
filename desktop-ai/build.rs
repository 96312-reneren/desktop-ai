fn main() {
    // Copy llama.dll to output directory for dynamic loading
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let dll_src = std::path::Path::new(&manifest_dir).join("llama.dll");

    if dll_src.exists() {
        // Copy to build output directory
        let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
        let target_dir = std::path::Path::new(&manifest_dir).join("target").join(&profile);

        // Also copy to deps directory for tests
        std::fs::create_dir_all(&target_dir).ok();
        let dll_dst = target_dir.join("llama.dll");
        if let Err(e) = std::fs::copy(&dll_src, &dll_dst) {
            if e.kind() != std::io::ErrorKind::AlreadyExists {
                eprintln!("Warning: failed to copy llama.dll: {}", e);
            }
        }
    }

    println!("cargo:rerun-if-changed=llama.dll");
    println!("cargo:rerun-if-changed=build.rs");
}
