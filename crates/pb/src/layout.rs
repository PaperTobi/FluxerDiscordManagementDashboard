//! Where the program's files and data are when nothing says otherwise (`PB_DATA`, `config.toml` and `PB__…` win).
//!
//! - Installed as the image does it (`/opt/pb/bin/pb`): the web page's files, the weights, espeak-ng's data and the
//!   shipped clips beside `bin/` (`/opt/pb/site`, …); the data in `/data`.
//! - Built in a source checkout (`<checkout>/target/release/pb`): what `cargo xtask web`, `cargo xtask espeak-ng` and
//!   `pb fetch-weights` put under `target/`, the checkout's `clips/`, and the data in `<checkout>/data`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub data: PathBuf,
    pub site: PathBuf,
    pub weights: PathBuf,
    pub espeak_data: PathBuf,
    pub clips: PathBuf,
}

impl Layout {
    /// The layout for a program at `exe`.
    pub fn of(exe: &Path) -> Layout {
        match checkout_of(exe) {
            Some(root) => Layout {
                data: root.join("data"),
                site: root.join("target/site"),
                weights: root.join("target/weights"),
                espeak_data: root.join("target/espeak-ng/share"),
                clips: root.join("clips"),
            },
            None => {
                // `<prefix>/bin/pb` → `<prefix>`
                let prefix = exe
                    .parent()
                    .and_then(Path::parent)
                    .map_or_else(|| PathBuf::from("/opt/pb"), Path::to_path_buf);
                Layout {
                    data: PathBuf::from("/data"),
                    site: prefix.join("site"),
                    weights: prefix.join("weights"),
                    espeak_data: prefix.join("espeak"),
                    clips: prefix.join("clips"),
                }
            }
        }
    }
}

/// The layout of this program.
pub fn here() -> &'static Layout {
    static HERE: OnceLock<Layout> = OnceLock::new();
    HERE.get_or_init(|| {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/opt/pb/bin/pb"));
        Layout::of(&exe)
    })
}

/// The source checkout a program was built in: the parent of the `target` directory it sits in, when that holds this
/// workspace.
fn checkout_of(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .filter(|d| d.file_name().is_some_and(|n| n == "target"))
        .filter_map(Path::parent)
        .find(|root| root.join("Cargo.toml").is_file() && root.join("crates/pb/Cargo.toml").is_file())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_files_sit_beside_bin_and_the_data_in_slash_data() {
        let l = Layout::of(Path::new("/opt/pb/bin/pb"));
        assert_eq!(l.data, PathBuf::from("/data"));
        assert_eq!(l.site, PathBuf::from("/opt/pb/site"));
        assert_eq!(l.weights, PathBuf::from("/opt/pb/weights"));
        assert_eq!(l.espeak_data, PathBuf::from("/opt/pb/espeak"));
        assert_eq!(l.clips, PathBuf::from("/opt/pb/clips"));
    }

    #[test]
    fn a_build_in_a_checkout_uses_the_checkout() {
        let root = std::env::temp_dir().join(format!("pb-layout-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("crates/pb")).unwrap_or_default();
        std::fs::write(root.join("Cargo.toml"), "").unwrap_or_default();
        std::fs::write(root.join("crates/pb/Cargo.toml"), "").unwrap_or_default();
        for exe in ["target/release/pb", "target/x86_64-unknown-linux-gnu/release/pb"] {
            let l = Layout::of(&root.join(exe));
            assert_eq!(l.data, root.join("data"), "{exe}");
            assert_eq!(l.site, root.join("target/site"));
            assert_eq!(l.weights, root.join("target/weights"));
            assert_eq!(l.espeak_data, root.join("target/espeak-ng/share"));
            assert_eq!(l.clips, root.join("clips"));
        }
        // A `target` directory that is not this workspace's is not a checkout.
        let other = Layout::of(&root.join("crates/target/release/pb"));
        assert_eq!(other.data, PathBuf::from("/data"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
