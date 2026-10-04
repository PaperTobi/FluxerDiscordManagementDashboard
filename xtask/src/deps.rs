//! Architecture rules between crates: layers, confinement of heavy third-party crates, purity of L0.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use cargo_metadata::{DependencyKind, MetadataCommand};
use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct Rules {
    layers: BTreeMap<String, Vec<String>>,
    allow: BTreeMap<String, Vec<String>>,
    /// Extra crates or layers dev-dependencies may use: `any` for every crate, or by layer.
    dev_allow: BTreeMap<String, Vec<String>>,
    confine: BTreeMap<String, Vec<String>>,
    l0_forbidden: L0Forbidden,
}

#[derive(Deserialize, Debug)]
struct L0Forbidden {
    crates: Vec<String>,
}

pub fn check(root: &Path) -> Result<()> {
    let rules: Rules = toml::from_str(&std::fs::read_to_string(root.join("xtask/layers.toml"))?)
        .context("reading xtask/layers.toml")?;
    let layer_of: BTreeMap<&str, &str> = rules
        .layers
        .iter()
        .flat_map(|(layer, crates)| crates.iter().map(move |c| (c.as_str(), layer.as_str())))
        .collect();
    let meta = MetadataCommand::new()
        .manifest_path(root.join("Cargo.toml"))
        .no_deps()
        .exec()?;
    let workspace: BTreeSet<&str> = meta.packages.iter().map(|p| p.name.as_str()).collect();

    let mut problems = Vec::new();
    for pkg in &meta.packages {
        let name = pkg.name.as_str();
        let Some(&layer) = layer_of.get(name) else {
            problems.push(format!("{name}: not assigned to a layer in xtask/layers.toml"));
            continue;
        };
        let allowed = rules.allow.get(layer).map(Vec::as_slice).unwrap_or_default();
        let permits = |dep: &str| {
            allowed
                .iter()
                .any(|a| a == dep || layer_of.get(dep).is_some_and(|l| l == a))
        };
        for dep in &pkg.dependencies {
            let dep_name = dep.name.as_str();
            let dev = dep.kind == DependencyKind::Development;
            if workspace.contains(dep_name) {
                let dev_ok = |key: &str| {
                    rules.dev_allow.get(key).is_some_and(|extra| {
                        extra
                            .iter()
                            .any(|a| a == dep_name || layer_of.get(dep_name).is_some_and(|l| l == a))
                    })
                };
                let ok = permits(dep_name) || (dev && (dev_ok("any") || dev_ok(layer)));
                if !ok {
                    let dep_layer = layer_of.get(dep_name).copied().unwrap_or("?");
                    problems.push(format!("{name} ({layer}) must not depend on {dep_name} ({dep_layer})"));
                }
                continue;
            }
            if !dev {
                if let Some(owners) = rules.confine.get(dep_name)
                    && !owners.iter().any(|o| o == name)
                {
                    problems.push(format!(
                        "{name} uses {dep_name}, which is confined to {}",
                        owners.join(", ")
                    ));
                }
                if layer == "L0" && rules.l0_forbidden.crates.iter().any(|c| c == dep_name) {
                    problems.push(format!("{name} is pure (L0) and must not use {dep_name}"));
                }
            }
        }
    }
    if problems.is_empty() {
        println!("deps: {} crates follow the layer rules", meta.packages.len());
        Ok(())
    } else {
        for p in &problems {
            eprintln!("deps: {p}");
        }
        bail!("{} architecture rule violation(s)", problems.len())
    }
}
