//! Lines of code per language in our own sources (crates/, xtask/, tools/), counted with tokei. Hand-written code must
//! be Rust; CSS, Fluent, TOML, Markdown and JSON test data are allowed; anything else must be in the exceptions register.

use std::path::Path;

use anyhow::{Result, bail};
use tokei::{Config, LanguageType, Languages};

const ALLOWED: &[LanguageType] = &[
    LanguageType::Rust,
    LanguageType::Css,
    LanguageType::Toml,
    LanguageType::Markdown,
    LanguageType::Json,
    LanguageType::Text,
];

/// A Fluent catalog: an `.ftl` file in a `locales` directory.
fn is_fluent(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "ftl") && path.components().any(|c| c.as_os_str() == "locales")
}

pub fn check(root: &Path) -> Result<()> {
    let register = super::zero_c::load_register(root)?;
    let dirs: Vec<_> = ["crates", "xtask", "tools"]
        .iter()
        .map(|d| root.join(d))
        .filter(|p| p.exists())
        .collect();
    let mut languages = Languages::new();
    languages.get_statistics(&dirs, &["target"], &Config::default());

    let mut problems = Vec::new();
    let mut rows: Vec<_> = languages.iter().filter(|(_, l)| !l.reports.is_empty()).collect();
    rows.sort_by_key(|(_, l)| std::cmp::Reverse(l.code));
    for (ty, lang) in &rows {
        println!(
            "langs: {:<12} {:>7} lines of code in {} files",
            ty.name(),
            lang.code,
            lang.reports.len()
        );
        if ALLOWED.contains(ty) {
            continue;
        }
        for report in &lang.reports {
            // tokei knows `.ftl` only as FreeMarker; ours are Fluent message catalogs (allowed).
            if **ty == LanguageType::FreeMarker && is_fluent(&report.name) {
                continue;
            }
            let rel = report
                .name
                .strip_prefix(root)
                .unwrap_or(&report.name)
                .to_string_lossy()
                .into_owned();
            if !register.exception.iter().any(|e| e.krate == rel) {
                problems.push(format!("{rel} is {} and not in the exceptions register", ty.name()));
            }
        }
    }
    if let Some((_, rust)) = rows.iter().find(|(t, _)| **t == LanguageType::Rust) {
        let total: usize = rows
            .iter()
            .filter(|(t, _)| {
                !matches!(
                    t,
                    LanguageType::Markdown | LanguageType::Json | LanguageType::Text | LanguageType::Toml
                )
            })
            .map(|(_, l)| l.code)
            .sum();
        if total > 0 {
            println!(
                "langs: Rust share of code {:.1}%",
                100.0 * rust.code as f64 / total as f64
            );
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        for p in &problems {
            eprintln!("langs: {p}");
        }
        bail!("{} file(s) in a language that is not allowed", problems.len())
    }
}
