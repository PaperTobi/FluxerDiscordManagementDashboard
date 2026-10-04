//! v1: [`MANIFEST`], [`fetch`], [`verify`].

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

// The pinned revisions, as macros so the download addresses below can be put together at compile time.
/// The classifier's Hugging Face revision.
macro_rules! roblox_revision {
    () => {
        "ddb1ffb03f2a53236b6d61dd846bfc286b1d4889"
    };
}
/// The Silero VAD commit (tag v6.2.1).
macro_rules! silero_commit {
    () => {
        "7e30209a3e901f9842f81b225f3e93d8199902b1"
    };
}
/// The rhasspy/piper-voices revision.
macro_rules! piper_revision {
    () => {
        "c10ece1aade47bb51c153c893d14e5bf8e5b7117"
    };
}

/// The classifier's Hugging Face revision (also written to `roblox-voice-safety-v3/REVISION`).
pub const ROBLOX_REVISION: &str = roblox_revision!();

/// One file to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Weight {
    /// Where it goes, relative to the weights directory.
    pub path: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    /// What it is.
    pub what: &'static str,
    /// Under which licence it is distributed (see its model card).
    pub licence: &'static str,
}

macro_rules! hf {
    ($repo:literal, $rev:expr, $($file:expr),+) => {
        concat!("https://huggingface.co/", $repo, "/resolve/", $rev, "/", $($file),+)
    };
}

macro_rules! voice {
    ($dir:literal, $name:literal, $onnx_size:literal, $onnx_sha:literal, $json_size:literal, $json_sha:literal, $licence:literal) => {
        [
            Weight {
                path: concat!("voices/", $name, "/", $name, ".onnx"),
                url: hf!("rhasspy/piper-voices", piper_revision!(), $dir, "/", $name, ".onnx"),
                sha256: $onnx_sha,
                size: $onnx_size,
                what: concat!("Piper voice ", $name),
                licence: $licence,
            },
            Weight {
                path: concat!("voices/", $name, "/", $name, ".onnx.json"),
                url: hf!(
                    "rhasspy/piper-voices",
                    piper_revision!(),
                    $dir,
                    "/",
                    $name,
                    ".onnx.json"
                ),
                sha256: $json_sha,
                size: $json_size,
                what: concat!("Piper voice ", $name, " (settings)"),
                licence: $licence,
            },
        ]
    };
}

const CLASSIFIER: [Weight; 3] = [
    Weight {
        path: "roblox-voice-safety-v3/model.safetensors",
        url: hf!(
            "Roblox/voice-safety-classifier-v3",
            roblox_revision!(),
            "model.safetensors"
        ),
        sha256: "fcd7fc7e764c7650fabb6812d2bc53ed90e5cfe68a9ecc6a37d1c47f06193b07",
        size: 1_279_671_844,
        what: "Roblox voice safety classifier v3 (weights)",
        licence: "Roblox model licence (LICENSE.md beside it)",
    },
    Weight {
        path: "roblox-voice-safety-v3/config.json",
        url: hf!("Roblox/voice-safety-classifier-v3", roblox_revision!(), "config.json"),
        sha256: "46358d6e6de0638c21b7e3de53a87ff790087dcf810fb252737af7f4641e2591",
        size: 1039,
        what: "Roblox voice safety classifier v3 (configuration)",
        licence: "Roblox model licence (LICENSE.md beside it)",
    },
    Weight {
        path: "roblox-voice-safety-v3/LICENSE.md",
        url: hf!("Roblox/voice-safety-classifier-v3", roblox_revision!(), "LICENSE.md"),
        sha256: "84b6fe6794ce2c637ad7f9ee499f487838f7974053271a1d600e1529c4128bd6",
        size: 11_348,
        what: "Roblox voice safety classifier v3 (licence)",
        licence: "-",
    },
];

const VAD: [Weight; 1] = [Weight {
    path: "silero-vad/silero_vad.onnx",
    url: concat!(
        "https://github.com/snakers4/silero-vad/raw/",
        silero_commit!(),
        "/src/silero_vad/data/silero_vad.onnx"
    ),
    sha256: "1a153a22f4509e292a94e67d6f9b85e8deb25b4988682b7e174c65279d8788e3",
    size: 2_327_524,
    what: "Silero VAD 6.2.1",
    licence: "MIT",
}];

const VOICES: [[Weight; 2]; 4] = [
    voice!(
        "de/de_DE/thorsten/high",
        "de_DE-thorsten-high",
        113_895_201,
        "9df1c43c61149ef9b39e618e2b861fbe41e1fcea9390b2dac62e8761573ea4f1",
        4875,
        "6de734444e4c3f9e33b7ebe2746dbc19b71e85f613e79c65acf623200b99a76a",
        "CC0-1.0 (Thorsten-Voice)"
    ),
    voice!(
        "de/de_DE/thorsten/medium",
        "de_DE-thorsten-medium",
        63_201_294,
        "7e64762d8e5118bb578f2eea6207e1a35a8e0c30595010b666f983fc87bb7819",
        4819,
        "974adee790533adb273a1ac88f49027d2a1b8f0f2cf4905954a4791e79264e85",
        "CC0-1.0 (Thorsten-Voice)"
    ),
    voice!(
        "en/en_US/lessac/high",
        "en_US-lessac-high",
        113_895_201,
        "4cabf7c3a638017137f34a1516522032d4fe3f38228a843cc9b764ddcbcd9e09",
        4883,
        "db42b97d9859f257bc1561b8ed980e7fb2398402050a74ddd6cbec931a92412f",
        "Blizzard 2013 Lessac licence (see the model card)"
    ),
    voice!(
        "en/en_US/lessac/medium",
        "en_US-lessac-medium",
        63_201_294,
        "5efe09e69902187827af646e1a6e9d269dee769f9877d17b16b1b46eeaaf019f",
        4885,
        "efe19c417bed055f2d69908248c6ba650fa135bc868b0e6abb3da181dab690a0",
        "Blizzard 2013 Lessac licence (see the model card)"
    ),
];

/// Every file, in download order (the classifier first: without it nothing works).
pub fn manifest() -> Vec<Weight> {
    let mut out: Vec<Weight> = CLASSIFIER.to_vec();
    out.extend(VAD);
    out.extend(VOICES.iter().flatten().copied());
    out
}

#[derive(Debug, thiserror::Error)]
pub enum WeightsError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{url}: {message}")]
    Download { url: String, message: String },
    #[error("{path}: the download does not match its pinned SHA-256 (got {got})")]
    Hash { path: String, got: String },
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> WeightsError + '_ {
    move |source| WeightsError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// How a file on disk compares with the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Ok,
    Missing,
    WrongSize(u64),
    WrongHash(String),
}

/// SHA-256 of a file, hex.
pub async fn sha256_file(path: &Path) -> std::io::Result<String> {
    use tokio::io::AsyncReadExt;
    let mut f = tokio::fs::File::open(path).await?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Checks one file (`deep` = hash it; otherwise only its size).
pub async fn check(dir: &Path, w: &Weight, deep: bool) -> State {
    let p = dir.join(w.path);
    let Ok(meta) = tokio::fs::metadata(&p).await else {
        return State::Missing;
    };
    if meta.len() != w.size {
        return State::WrongSize(meta.len());
    }
    if deep {
        match sha256_file(&p).await {
            Ok(h) if h == w.sha256 => State::Ok,
            Ok(h) => State::WrongHash(h),
            Err(_) => State::Missing,
        }
    } else {
        State::Ok
    }
}

/// Every file's state.
pub async fn verify(dir: &Path, deep: bool) -> Vec<(Weight, State)> {
    let mut out = Vec::new();
    for w in manifest() {
        let s = check(dir, &w, deep).await;
        out.push((w, s));
    }
    out
}

/// How long a download may deliver nothing before it counts as stalled.
const STALL: std::time::Duration = std::time::Duration::from_secs(60);

/// Progress of a download.
#[derive(Debug, Clone, Copy)]
pub struct Progress<'a> {
    pub weight: &'a Weight,
    pub done: u64,
}

/// Downloads every file that is missing or wrong into `dir`. A download continues where a previous one stopped
/// (`<file>.part`), is checked against its size and SHA-256, and replaces the file only when it matches. Writes the
/// classifier's `REVISION` file.
pub async fn fetch(
    dir: &Path,
    http: &reqwest::Client,
    mut progress: impl FnMut(Progress<'_>),
) -> Result<(), WeightsError> {
    for w in manifest() {
        if check(dir, &w, true).await == State::Ok {
            tracing::info!(file = w.path, "already there");
            continue;
        }
        download(dir, http, &w, &mut progress).await?;
    }
    let rev = dir.join("roblox-voice-safety-v3/REVISION");
    tokio::fs::write(&rev, format!("{ROBLOX_REVISION}\n"))
        .await
        .map_err(io(&rev))?;
    Ok(())
}

async fn download(
    dir: &Path,
    http: &reqwest::Client,
    w: &Weight,
    progress: &mut impl FnMut(Progress<'_>),
) -> Result<(), WeightsError> {
    let dest = dir.join(w.path);
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(io(parent))?;
    }
    let part = {
        let mut p = dest.clone().into_os_string();
        p.push(".part");
        PathBuf::from(p)
    };
    let mut have = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
    if have > w.size {
        let _ = tokio::fs::remove_file(&part).await;
        have = 0;
    }
    let fail = |message: String| WeightsError::Download {
        url: w.url.to_owned(),
        message,
    };
    if have < w.size {
        let mut req = http.get(w.url);
        if have > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let mut res = req.send().await.map_err(|e| fail(e.to_string()))?;
        let status = res.status();
        // A server that ignores the range sends everything again: start over.
        if have > 0 && status != reqwest::StatusCode::PARTIAL_CONTENT {
            have = 0;
        }
        if !status.is_success() {
            return Err(fail(format!("HTTP {status}")));
        }
        let mut f = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(have > 0)
            .truncate(have == 0)
            .open(&part)
            .await
            .map_err(io(&part))?;
        let mut done = have;
        // A connection that delivers nothing for a minute has stalled; the next run continues from the `.part` file.
        while let Some(chunk) = tokio::time::timeout(STALL, res.chunk())
            .await
            .map_err(|_| fail(format!("nothing arrived for {} s", STALL.as_secs())))?
            .map_err(|e| fail(e.to_string()))?
        {
            f.write_all(&chunk).await.map_err(io(&part))?;
            done += chunk.len() as u64;
            progress(Progress { weight: w, done });
        }
        f.sync_all().await.map_err(io(&part))?;
    }
    let got = sha256_file(&part).await.map_err(io(&part))?;
    if got != w.sha256 {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(WeightsError::Hash {
            path: w.path.to_owned(),
            got,
        });
    }
    tokio::fs::rename(&part, &dest).await.map_err(io(&dest))?;
    tracing::info!(file = w.path, "downloaded and checked");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_is_complete_and_pinned() {
        let m = manifest();
        assert_eq!(m.len(), 3 + 1 + 8);
        for w in &m {
            assert_eq!(w.sha256.len(), 64, "{}", w.path);
            assert!(w.sha256.bytes().all(|b| b.is_ascii_hexdigit()), "{}", w.path);
            assert!(w.size > 0);
            let pinned = [roblox_revision!(), silero_commit!(), piper_revision!()];
            assert!(pinned.iter().any(|p| w.url.contains(p)), "{} is not pinned", w.url);
        }
    }

    #[tokio::test]
    async fn checks_what_is_on_disk() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let w = manifest()[2];
        assert_eq!(check(dir.path(), &w, true).await, State::Missing);
        let p = dir.path().join(w.path);
        tokio::fs::create_dir_all(p.parent().unwrap_or(dir.path())).await.ok();
        tokio::fs::write(&p, b"short").await.ok();
        assert_eq!(check(dir.path(), &w, true).await, State::WrongSize(5));
    }
}
