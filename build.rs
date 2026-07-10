use std::{env, fs, path::PathBuf};

fn main() {
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_env != "musl" {
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
