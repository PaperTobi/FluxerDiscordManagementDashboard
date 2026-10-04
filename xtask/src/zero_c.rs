//! Zero-C gate: walks the dependency graph of the shipped binary (normal and build dependencies, Linux x86-64) and
//! flags every crate that links a native library or compiles C/C++ in a build script. Each flag must be covered by
//! docs/exceptions.toml: scope "shipped" for a crate that ends up in the binary, "build" (or "shipped") for one that
//! only runs while building. Entries past their review date fail too.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use std::process::Command;

use cargo_metadata::MetadataCommand;
use serde::Deserialize;

const SHIPPED_ROOTS: &[&str] = &["pb"];
const C_BUILD_TOOLS: &[&str] = &[
    "cc",
    "cmake",
    "bindgen",
    "pkg-config",
    "cxx-build",
    "autotools",
    "nasm-rs",
    "vcpkg",
];

#[derive(Deserialize, Debug)]
pub struct Register {
    #[serde(default)]
    pub exception: Vec<Exception>,
    /// Crates with a `links` key that were checked by hand and contain no native code.
    #[serde(default)]
    pub verified_pure: Vec<VerifiedPure>,
    /// Vendored dependencies with local changes.
    #[serde(default)]
    pub patched: Vec<Patched>,
}

#[derive(Deserialize, Debug)]
pub struct Patched {
    #[serde(rename = "crate")]
    pub krate: String,
    pub version: String,
    pub path: String,
    pub reason: String,
    pub upstream: String,
    pub review: String,
}

/// A crate whose `links` key or C build tool was checked by hand: at this version and with our features, nothing native
/// is built or linked.
#[derive(Deserialize, Debug)]
pub struct VerifiedPure {
    #[serde(rename = "crate")]
    pub krate: String,
    /// The version that was checked (another version must be checked again).
    pub version: String,
    pub note: String,
}

#[derive(Deserialize, Debug)]
pub struct Exception {
    #[serde(rename = "crate")]
    pub krate: String,
    pub kind: String,
    pub scope: String,
    pub reason: String,
    pub cost: String,
    pub review: String,
}

pub fn load_register(root: &Path) -> Result<Register> {
    toml::from_str(&std::fs::read_to_string(root.join("docs/exceptions.toml"))?).context("reading docs/exceptions.toml")
}

pub fn check(root: &Path) -> Result<()> {
    let register = load_register(root)?;
    let today = jiff::Zoned::now().date();
    let mut problems = Vec::new();
    for e in &register.exception {
        if e.reason.trim().is_empty() || e.cost.trim().is_empty() || e.kind.trim().is_empty() {
            problems.push(format!("exception {}: reason, cost and kind are required", e.krate));
        }
        match e.review.parse::<jiff::civil::Date>() {
            Ok(d) if d < today => problems.push(format!("exception {}: review date {} has passed", e.krate, e.review)),
            Ok(_) => {}
            Err(err) => problems.push(format!("exception {}: bad review date {:?}: {err}", e.krate, e.review)),
        }
    }

    for p in &register.patched {
        let dir = root.join(&p.path);
        if !dir.join("PATCHES.md").exists() {
            problems.push(format!(
                "patched {}: {} has no PATCHES.md describing the changes",
                p.krate, p.path
            ));
        }
        match p.review.parse::<jiff::civil::Date>() {
            Ok(d) if d < today => problems.push(format!("patched {}: review date {} has passed", p.krate, p.review)),
            Ok(_) => {}
            Err(err) => problems.push(format!("patched {}: bad review date {:?}: {err}", p.krate, p.review)),
        }
        if p.reason.trim().is_empty() || p.upstream.trim().is_empty() {
            problems.push(format!("patched {}: reason and upstream are required", p.krate));
        }
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap_or_default();
        if !manifest.contains(&format!("version = \"{}\"", p.version)) {
            problems.push(format!("patched {}: {} is not version {}", p.krate, p.path, p.version));
        }
        println!("zero-c: patched dependency {} {} at {}", p.krate, p.version, p.path);
    }

    // The graph Cargo really builds (per-dependency feature resolution), from `cargo tree`; `cargo metadata` merges
    // features across the whole workspace and would report C that is never compiled.
    let tree = shipped_tree(root, "normal,build")?;
    let linked = shipped_tree(root, "normal")?;
    let meta = MetadataCommand::new().manifest_path(root.join("Cargo.toml")).exec()?;
    let links: BTreeMap<(String, String), Option<String>> = meta
        .packages
        .iter()
        .map(|p| ((p.name.to_string(), p.version.to_string()), p.links.clone()))
        .collect();

    let pure: BTreeMap<&str, &str> = register
        .verified_pure
        .iter()
        .map(|p| (p.krate.as_str(), p.version.as_str()))
        .collect();
    for p in &register.verified_pure {
        if p.note.trim().is_empty() {
            problems.push(format!(
                "verified_pure {}: a note on how it was checked is required",
                p.krate
            ));
        }
    }
    let scope_of: BTreeMap<&str, &str> = register
        .exception
        .iter()
        .map(|e| (e.krate.as_str(), e.scope.as_str()))
        .collect();
    let mut flagged = 0usize;
    for (pkg, children) in &tree {
        let (name, version) = pkg;
        if C_BUILD_TOOLS.contains(&name.as_str()) {
            continue; // the tools themselves; the crates that use them are what gets flagged
        }
        let builds_c: Vec<&str> = children
            .iter()
            .filter(|c| C_BUILD_TOOLS.contains(&c.0.as_str()))
            .map(|c| c.0.as_str())
            .collect();
        let mut why = Vec::new();
        if let Some(Some(lib)) = links.get(pkg) {
            why.push(format!("declares `links = \"{lib}\"`"));
        }
        why.extend(builds_c.iter().map(|tool| format!("builds with `{tool}`")));
        match pure.get(name.as_str()) {
            Some(v) if *v == version.as_str() && !why.is_empty() => {
                println!("zero-c: verified pure {name} {version} ({})", why.join(", "));
                continue;
            }
            Some(v) if !why.is_empty() => {
                problems.push(format!(
                    "verified_pure {name}: checked at {v}, the graph has {version}; check it again"
                ));
                continue;
            }
            _ => {}
        }
        if why.is_empty() {
            if name.ends_with("-sys") {
                println!("zero-c: OS binding, pure Rust: {name} {version}");
            }
            continue;
        }
        flagged += 1;
        let needed = if linked.contains_key(pkg) { "shipped" } else { "build" };
        let line = format!("{name} {version} ({needed}): {}", why.join(", "));
        match scope_of.get(name.as_str()) {
            Some(&"shipped") => println!("zero-c: excepted  {line}"),
            Some(&"build") if needed == "build" => println!("zero-c: excepted  {line}"),
            Some(s) => problems.push(format!("excepted with scope {s:?} but needed as {needed:?}: {line}")),
            None => problems.push(format!("not in the exceptions register: {line}")),
        }
    }
    // An entry for a crate the graph no longer has is stale: the register lists only what is used.
    let in_graph = |krate: &str| tree.keys().any(|(n, _)| n == krate);
    for e in register.exception.iter().filter(|e| e.scope != "dev") {
        if !in_graph(&e.krate) {
            problems.push(format!(
                "exception {}: not in the graph any more; remove the entry",
                e.krate
            ));
        }
    }
    for p in &register.verified_pure {
        if !in_graph(&p.krate) {
            problems.push(format!(
                "verified_pure {}: not in the graph any more; remove the entry",
                p.krate
            ));
        }
    }
    let seen = &tree;
    println!(
        "zero-c: {} crates in the shipped graph, {flagged} with native code",
        seen.len()
    );
    if problems.is_empty() {
        Ok(())
    } else {
        for p in &problems {
            eprintln!("zero-c: {p}");
        }
        bail!("{} zero-C problem(s)", problems.len())
    }
}

type Pkg = (String, String);

/// Every package in the shipped binaries' graph over `edges` (`normal`: what is linked in; `normal,build`: also what
/// runs while building), Linux x86-64, with its direct dependencies, as Cargo resolves it.
fn shipped_tree(root: &Path, edges: &str) -> Result<BTreeMap<Pkg, BTreeSet<Pkg>>> {
    let mut graph: BTreeMap<Pkg, BTreeSet<Pkg>> = BTreeMap::new();
    for krate in SHIPPED_ROOTS {
        let out = Command::new("cargo")
            .args(["tree", "-p", krate, "-e", edges, "--target", "x86_64-unknown-linux-gnu"])
            .args(["--prefix", "depth", "--format", "{p}"])
            .current_dir(root)
            .output()
            .context("running cargo tree")?;
        if !out.status.success() {
            bail!("cargo tree failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        let mut stack: Vec<Pkg> = Vec::new();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let digits = line.chars().take_while(char::is_ascii_digit).count();
            let Ok(depth) = line[..digits].parse::<usize>() else {
                continue;
            };
            let mut words = line[digits..].split_whitespace();
            let (Some(name), Some(version)) = (words.next(), words.next()) else {
                continue;
            };
            let pkg: Pkg = (name.to_owned(), version.trim_start_matches('v').to_owned());
            graph.entry(pkg.clone()).or_default();
            stack.truncate(depth);
            if let Some(parent) = stack.last() {
                graph.entry(parent.clone()).or_default().insert(pkg.clone());
            }
            stack.push(pkg);
        }
    }
    Ok(graph)
}
