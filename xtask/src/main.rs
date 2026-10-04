//! `cargo xtask <check>`: the project's own gates (docs/design.md §0, §1, §9).

mod deps;
mod espeak;
mod freshness;
mod langs;
mod shipped_js;
mod web;
mod zero_c;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(about = "Project checks")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Everything CI runs: fmt, clippy (native and wasm), the browser bundle, tests, cargo-deny, cargo-shear (unused
    /// dependencies), deps, zero-c, langs, shipped-js.
    Ci,
    /// Layer and confinement rules between crates (xtask/layers.toml).
    Deps,
    /// No C/C++ in the shipped program except what docs/exceptions.toml lists.
    ZeroC,
    /// Lines of code per language in our own sources; fails on languages that are not allowed.
    Langs,
    /// Lists every JavaScript file the browser receives; fails on anything hand-written.
    ShippedJs,
    /// Builds the pinned espeak-ng C library (the phonemizer Piper's voices were trained with) into
    /// target/espeak-ng-<commit>; pb-espeak links it from there (or from PB_ESPEAK_NG).
    EspeakNg,
    /// Is every direct dependency on its newest release and still maintained (crates.io and GitHub)?
    Freshness,
    /// Builds the browser bundle into target/site/pkg: the app's wasm (pb-web, `hydrate`), its wasm-bindgen loader and
    /// the stylesheet.
    Web {
        /// A quick unoptimized build.
        #[arg(long)]
        dev: bool,
    },
}

fn main() -> Result<()> {
    let root = workspace_root()?;
    match Cli::parse().cmd {
        Cmd::Ci => ci(&root),
        Cmd::Deps => deps::check(&root),
        Cmd::ZeroC => zero_c::check(&root),
        Cmd::Langs => langs::check(&root),
        Cmd::ShippedJs => shipped_js::check(&root),
        Cmd::Freshness => freshness::check(&root),
        Cmd::Web { dev } => web::build(&root, dev),
        Cmd::EspeakNg => espeak::build(&root).map(|prefix| println!("espeak-ng installed in {}", prefix.display())),
    }
}

fn workspace_root() -> Result<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    dir.parent()
        .map(Path::to_path_buf)
        .context("xtask must live in the workspace root")
}

fn ci(root: &Path) -> Result<()> {
    run(root, "cargo", &["fmt", "--all", "--check"])?;
    run(
        root,
        "cargo",
        &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"],
    )?;
    // The browser side of pb-web only compiles for wasm with `hydrate`; the workspace pass above never sees it.
    run(
        root,
        "cargo",
        &[
            "clippy",
            "-p",
            "pb-web",
            "--lib",
            "--features",
            "hydrate",
            "--target",
            "wasm32-unknown-unknown",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    web::build(root, false)?;
    run(root, "cargo", &["test", "--workspace"])?;
    run(root, "cargo", &["deny", "check"])?;
    run(root, "cargo", &["shear"])?;
    deps::check(root)?;
    zero_c::check(root)?;
    langs::check(root)?;
    shipped_js::check(root)?;
    println!("ci: all checks passed");
    Ok(())
}

fn run(root: &Path, program: &str, args: &[&str]) -> Result<()> {
    println!("$ {program} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(root)
        .status()
        .with_context(|| format!("starting {program}"))?;
    if !status.success() {
        bail!("{program} {} failed ({status})", args.join(" "));
    }
    Ok(())
}
