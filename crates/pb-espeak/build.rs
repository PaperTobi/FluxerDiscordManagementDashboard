//! Links the static espeak-ng built by `cargo xtask espeak-ng` (or the install prefix in `PB_ESPEAK_NG`), at the
//! commit `espeak-ng.commit` pins.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=PB_ESPEAK_NG");
    println!("cargo:rerun-if-changed=espeak-ng.commit");
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let prefix = match std::env::var_os("PB_ESPEAK_NG") {
        Some(p) => PathBuf::from(p),
        None => {
            // The build `cargo xtask espeak-ng` made must be of the pinned commit.
            let prefix = manifest.join("../../target/espeak-ng");
            let pinned = std::fs::read_to_string(manifest.join("espeak-ng.commit")).unwrap_or_default();
            let built = std::fs::read_to_string(prefix.join("COMMIT")).unwrap_or_default();
            println!("cargo:rerun-if-changed={}", prefix.join("COMMIT").display());
            if built.trim() != pinned.trim() {
                panic!(
                    "espeak-ng at {} is not of commit {}: run `cargo xtask espeak-ng`",
                    prefix.display(),
                    pinned.trim()
                );
            }
            prefix
        }
    };
    let lib = prefix.join("lib");
    if !lib.join("libespeak-ng.a").exists() || !lib.join("libucd.a").exists() {
        panic!(
            "espeak-ng is not built at {}: run `cargo xtask espeak-ng` (or set PB_ESPEAK_NG to its install prefix)",
            prefix.display()
        );
    }
    println!("cargo:rerun-if-changed={}", lib.join("libespeak-ng.a").display());
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=static=espeak-ng");
    println!("cargo:rustc-link-lib=static=ucd");
    println!("cargo:rustc-link-lib=dylib=m");
    println!(
        "cargo:rustc-env=PB_ESPEAK_NG_BUILD_DATA={}",
        prefix.join("share").display()
    );
}
