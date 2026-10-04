//! `cargo xtask web`: the browser bundle. Cargo builds pb-web for wasm32 with the `hydrate` feature; wasm-bindgen (as a
//! library, the same version the app links) writes the module and its generated loader; the stylesheet is copied
//! beside them. The server serves target/site/pkg as `/pkg`.

use std::path::Path;

use anyhow::{Context, Result};

pub fn build(root: &Path, dev: bool) -> Result<()> {
    let (profile, dir) = if dev { ("dev", "debug") } else { ("wasm", "wasm") };
    super::run(
        root,
        "cargo",
        &[
            "build",
            "-p",
            "pb-web",
            "--lib",
            "--features",
            "hydrate",
            "--target",
            "wasm32-unknown-unknown",
            "--profile",
            profile,
        ],
    )?;
    let wasm = root.join("target/wasm32-unknown-unknown").join(dir).join("pb_web.wasm");
    let pkg = root.join("target/site/pkg");
    std::fs::create_dir_all(&pkg).with_context(|| format!("creating {}", pkg.display()))?;
    wasm_bindgen_cli_support::Bindgen::new()
        .input_path(&wasm)
        .web(true)?
        .typescript(false)
        .debug(dev)
        .keep_debug(dev)
        .out_name("pb")
        .generate(&pkg)
        .with_context(|| format!("wasm-bindgen on {}", wasm.display()))?;
    let css = root.join("crates/pb-web/style/app.css");
    std::fs::copy(&css, pkg.join("pb.css")).with_context(|| format!("copying {}", css.display()))?;
    for f in ["pb.js", "pb_bg.wasm", "pb.css"] {
        let size = std::fs::metadata(pkg.join(f)).map(|m| m.len()).unwrap_or(0);
        println!("web: target/site/pkg/{f} ({size} bytes)");
    }
    super::shipped_js::check(root)
}
