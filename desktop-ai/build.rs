fn llama_library_name() -> &'static str {
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => "llama.dll",
        Ok("macos") => "libllama.dylib",
        _ => "libllama.so",
    }
}

/// Additional shared libraries that the platform llama library links
/// against, copied next to it when present (self-contained package).
fn ggml_library_names() -> Vec<&'static str> {
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => vec![
            "ggml.dll",
            "ggml-base.dll",
            "ggml-cpu.dll",
            "libgcc_s_seh-1.dll",
            "libstdc++-6.dll",
            "libwinpthread-1.dll",
            "libgomp-1.dll",
        ],
        Ok("macos") => Vec::new(),
        _ => vec!["libggml.so.0", "libggml-cpu.so.0", "libggml-base.so.0"],
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
    println!("cargo:rerun-if-changed=ggml.dll");
    println!("cargo:rerun-if-changed=ggml-base.dll");
    println!("cargo:rerun-if-changed=ggml-cpu.dll");
    println!("cargo:rerun-if-changed=libgcc_s_seh-1.dll");
    println!("cargo:rerun-if-changed=libstdc++-6.dll");
    println!("cargo:rerun-if-changed=libwinpthread-1.dll");
    println!("cargo:rerun-if-changed=libgomp-1.dll");
    println!("cargo:rerun-if-changed=libllama.so");
    println!("cargo:rerun-if-changed=libllama.dylib");
    println!("cargo:rerun-if-changed=libggml.so.0");
    println!("cargo:rerun-if-changed=libggml-cpu.so.0");
    println!("cargo:rerun-if-changed=libggml-base.so.0");
    println!("cargo:rerun-if-changed=build.rs");
}
