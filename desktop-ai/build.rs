fn llama_library_name() -> &'static str {
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => "llama.dll",
        Ok("macos") => "libllama.dylib",
        _ => "libllama.so",
    }
}

/// On Linux, libllama.so links against the ggml shared libraries, so copy
/// those next to it when present (self-contained package, no ldconfig needed).
fn ggml_library_names() -> Vec<&'static str> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        Vec::new()
    } else {
        vec!["libggml.so.0", "libggml-cpu.so.0", "libggml-base.so.0"]
    }
}

fn main() {
    // Copy the platform llama shared library to the output directory for
    // dynamic loading (llama.dll / libllama.dylib / libllama.so).
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let target_dir = std::path::Path::new(&manifest_dir)
        .join("target")
        .join(&profile);

    std::fs::create_dir_all(&target_dir).ok();

    let mut lib_names = vec![llama_library_name()];
    lib_names.extend(ggml_library_names());

    for lib_name in lib_names {
        let lib_src = std::path::Path::new(&manifest_dir).join(lib_name);
        if lib_src.exists() {
            let lib_dst = target_dir.join(lib_name);
            if let Err(e) = std::fs::copy(&lib_src, &lib_dst) {
                if e.kind() != std::io::ErrorKind::AlreadyExists {
                    eprintln!("Warning: failed to copy {}: {}", lib_name, e);
                }
            }
        }
    }

    println!("cargo:rerun-if-changed=llama.dll");
    println!("cargo:rerun-if-changed=libllama.so");
    println!("cargo:rerun-if-changed=libllama.dylib");
    println!("cargo:rerun-if-changed=libggml.so.0");
    println!("cargo:rerun-if-changed=libggml-cpu.so.0");
    println!("cargo:rerun-if-changed=libggml-base.so.0");
    println!("cargo:rerun-if-changed=build.rs");
}
