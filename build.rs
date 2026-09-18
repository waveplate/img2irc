use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let build_number_file = Path::new(&manifest_dir).join("build_number");

    let current_build_num: u64 = fs::read_to_string(&build_number_file)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    let next_build_num = current_build_num + 1;
    if let Err(err) = fs::write(&build_number_file, format!("{}\n", next_build_num)) {
        println!("cargo:warning=failed to write build_number: {err}");
    }

    let pkg_version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".to_string());
    let base_version = pkg_version.split('+').next().unwrap_or(&pkg_version);
    let full_version = format!("{}+{}", base_version, next_build_num);

    println!("cargo:rustc-env=BUILD_NUMBER={}", next_build_num);
    println!("cargo:rustc-env=IMG2IRC_BUILD_NUMBER={}", next_build_num);
    println!("cargo:rustc-env=IMG2IRC_VERSION={}", full_version);

    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let ocr_enabled = env::var_os("CARGO_FEATURE_OCR").is_some();
    if target_env != "musl" || !ocr_enabled {
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let libdl = out_dir.join("libdl.a");

    if let Err(err) = fs::write(&libdl, b"!<arch>\n") {
        println!("cargo:warning=failed to create musl libdl compatibility archive: {err}");
        return;
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
}
