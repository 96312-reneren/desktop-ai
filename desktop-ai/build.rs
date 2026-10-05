fn llama_library_name() -> &'static str {
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => "llama.dll",
        Ok("macos") => "libllama.dylib",
        _ => "libllama.so",
    }
}

/// Directory under `vendor/` for the current target platform.
fn vendor_platform_dir() -> &'static str {
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => "windows",
        Ok("macos") => "macos",
        _ => "linux",
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
        // Linux: CI builds these from llama.cpp b7700 and vendors them next
        // to the crate (vendor/linux/); a local Linux build may also supply
        // its own copies there.
        _ => vec!["libggml.so.0", "libggml-cpu.so.0", "libggml-base.so.0"],
    }
}

fn main() {
    // Copy the platform llama shared library (and its ggml companions) to the
    // output directory for dynamic loading. Sources live in
    // `<repo>/vendor/<platform>/`; the old in-crate location is still checked
    // as a fallback so local Linux builds that drop .so files next to
    // Cargo.toml keep working.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let target_dir = std::path::Path::new(&manifest_dir)
        .join("target")
        .join(&profile);

    std::fs::create_dir_all(&target_dir).ok();

    let vendor_dir = std::path::Path::new(&manifest_dir)
        .join("..")
        .join("vendor")
        .join(vendor_platform_dir());

    let mut lib_names = vec![llama_library_name()];
    lib_names.extend(ggml_library_names());

    for lib_name in lib_names {
        let candidates = [
            vendor_dir.join(lib_name),
            std::path::Path::new(&manifest_dir).join(lib_name),
        ];
        let Some(lib_src) = candidates.iter().find(|p| p.exists()) else {
            continue;
        };
        let lib_dst = target_dir.join(lib_name);
        if let Err(e) = std::fs::copy(lib_src, &lib_dst) {
            if e.kind() != std::io::ErrorKind::AlreadyExists {
                eprintln!("Warning: failed to copy {}: {}", lib_name, e);
            }
        }
    }

    println!("cargo:rerun-if-changed=../vendor/windows/llama.dll");
    println!("cargo:rerun-if-changed=../vendor/windows/ggml.dll");
    println!("cargo:rerun-if-changed=../vendor/windows/ggml-base.dll");
    println!("cargo:rerun-if-changed=../vendor/windows/ggml-cpu.dll");
    println!("cargo:rerun-if-changed=../vendor/windows/libgcc_s_seh-1.dll");
    println!("cargo:rerun-if-changed=../vendor/windows/libstdc++-6.dll");
    println!("cargo:rerun-if-changed=../vendor/windows/libwinpthread-1.dll");
    println!("cargo:rerun-if-changed=../vendor/windows/libgomp-1.dll");
    println!("cargo:rerun-if-changed=../vendor/linux/libllama.so");
    println!("cargo:rerun-if-changed=../vendor/linux/libggml.so.0");
    println!("cargo:rerun-if-changed=../vendor/linux/libggml-cpu.so.0");
    println!("cargo:rerun-if-changed=../vendor/linux/libggml-base.so.0");
    println!("cargo:rerun-if-changed=build.rs");
}
