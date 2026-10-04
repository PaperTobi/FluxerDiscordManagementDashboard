//! Builds espeak-ng at the commit piper-tts 1.8.0 builds (its `CMakeLists.txt` `GIT_TAG`), pinned in
//! `crates/pb-espeak/espeak-ng.commit`, so the phonemes are exactly the ones the Piper voices were trained with.
//! Static library, no audio output, no optional extras. Installed into `target/espeak-ng` (its `COMMIT` file says
//! which commit it is).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub const REPO: &str = "https://github.com/espeak-ng/espeak-ng.git";
/// The pinned commit.
fn commit() -> &'static str {
    include_str!("../../crates/pb-espeak/espeak-ng.commit").trim()
}

fn run(cmd: &mut Command) -> Result<()> {
    let shown = format!("{cmd:?}");
    let status = cmd.status().with_context(|| format!("starting {shown}"))?;
    if !status.success() {
        bail!("{shown} failed ({status})");
    }
    Ok(())
}

/// Builds (if needed) and returns the install prefix.
pub fn build(root: &Path) -> Result<PathBuf> {
    let target = root.join("target");
    let prefix = target.join("espeak-ng");
    let built = std::fs::read_to_string(prefix.join("COMMIT")).unwrap_or_default();
    if built.trim() == commit()
        && prefix.join("lib/libucd.a").exists()
        && prefix.join("share/espeak-ng-data/phontab").exists()
    {
        return Ok(prefix);
    }
    if prefix.exists() {
        std::fs::remove_dir_all(&prefix).with_context(|| format!("removing the old {}", prefix.display()))?;
    }
    let src = target.join("espeak-ng-src");
    if !src.join(".git").exists() {
        run(Command::new("git").args(["clone", "--quiet", REPO]).arg(&src))?;
    }
    run(Command::new("git")
        .arg("-C")
        .arg(&src)
        .args(["fetch", "--quiet", "origin"]))?;
    run(Command::new("git")
        .arg("-C")
        .arg(&src)
        .args(["checkout", "--quiet", commit()]))?;
    let build = src.join("build");
    run(Command::new("cmake")
        .arg("-S")
        .arg(&src)
        .arg("-B")
        .arg(&build)
        .args(["-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release", "-DBUILD_SHARED_LIBS=OFF"])
        .arg(format!("-DCMAKE_INSTALL_PREFIX={}", prefix.display()))
        .args([
            "-DUSE_ASYNC=OFF",
            "-DUSE_MBROLA=OFF",
            "-DUSE_LIBSONIC=OFF",
            "-DUSE_LIBPCAUDIO=OFF",
        ])
        .args([
            "-DUSE_KLATT=OFF",
            "-DUSE_SPEECHPLAYER=OFF",
            "-DEXTRA_cmn=ON",
            "-DEXTRA_ru=ON",
        ])
        .args([
            "-DCMAKE_C_FLAGS=-D_FILE_OFFSET_BITS=64",
            "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
        ]))?;
    run(Command::new("ninja").arg("-C").arg(&build))?;
    run(Command::new("ninja").arg("-C").arg(&build).arg("install"))?;
    std::fs::copy(build.join("src/ucd-tools/libucd.a"), prefix.join("lib/libucd.a"))
        .context("copying libucd.a (espeak-ng's Unicode tables)")?;
    std::fs::write(prefix.join("COMMIT"), format!("{}\n", commit())).context("writing COMMIT")?;
    Ok(prefix)
}
