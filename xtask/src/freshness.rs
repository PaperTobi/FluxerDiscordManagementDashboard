//! `cargo xtask freshness`: is every direct dependency on its newest release, and is it still maintained?
//!
//! For each crate our manifests name (from crates.io): the locked version, the newest stable release and its age;
//! from its repository (GitHub, best effort; `GITHUB_TOKEN` raises the rate limit): archived or missing, last push.
//! Fails on a newer release the version requirement does not allow, an archived or missing repository, or a crate
//! with neither a release nor a push for a year, unless `xtask/freshness.toml` explains why. Unmaintained crates
//! deeper in the tree are `cargo deny check advisories`' job.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use cargo_metadata::MetadataCommand;
use cargo_metadata::semver::{Version, VersionReq};
use serde::Deserialize;

const STALE_DAYS: i64 = 365;

#[derive(Deserialize, Debug, Default)]
struct Allow {
    #[serde(default)]
    allow: Vec<AllowEntry>,
}

#[derive(Deserialize, Debug)]
struct AllowEntry {
    #[serde(rename = "crate")]
    krate: String,
    reason: String,
}

#[derive(Deserialize, Debug)]
struct CrateResponse {
    #[serde(rename = "crate")]
    krate: CrateInfo,
    versions: Vec<VersionInfo>,
}

#[derive(Deserialize, Debug)]
struct CrateInfo {
    max_stable_version: Option<String>,
    repository: Option<String>,
}

#[derive(Deserialize, Debug)]
struct VersionInfo {
    created_at: String,
    yanked: bool,
}

#[derive(Deserialize, Debug)]
struct Repo {
    archived: bool,
    pushed_at: Option<String>,
}

#[derive(Clone)]
enum RepoState {
    Ok { days_since_push: i64 },
    Archived,
    Missing,
    Unknown(String),
}

fn days_since(ts: &str) -> Option<i64> {
    let t: jiff::Timestamp = ts.parse().ok()?;
    Some(jiff::Timestamp::now().duration_since(t).as_hours() / 24)
}

fn github(url: &str) -> Option<(String, String)> {
    let rest = url.trim_end_matches(".git").split("github.com/").nth(1)?;
    let mut parts = rest.split('/');
    Some((parts.next()?.to_owned(), parts.next()?.to_owned()))
}

pub fn check(root: &Path) -> Result<()> {
    let allow: Allow = match std::fs::read_to_string(root.join("xtask/freshness.toml")) {
        Ok(t) => toml::from_str(&t).context("reading xtask/freshness.toml")?,
        Err(_) => Allow::default(),
    };
    let allowed: BTreeMap<&str, &str> = allow
        .allow
        .iter()
        .map(|a| (a.krate.as_str(), a.reason.as_str()))
        .collect();

    let meta = MetadataCommand::new().manifest_path(root.join("Cargo.toml")).exec()?;
    let members: BTreeSet<_> = meta.workspace_members.iter().collect();
    // Every crates.io dependency of our crates with all the requirements we state for it.
    let mut reqs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for p in meta.packages.iter().filter(|p| members.contains(&p.id)) {
        for d in &p.dependencies {
            if d.source.as_ref().is_some_and(|s| s.repr.starts_with("registry+")) {
                reqs.entry(d.name.clone()).or_default().insert(d.req.to_string());
            }
        }
    }
    let mut locked: BTreeMap<&str, Vec<&Version>> = BTreeMap::new();
    for p in &meta.packages {
        locked.entry(p.name.as_str()).or_default().push(&p.version);
    }

    let tls = pb_tls::client_config().map_err(|e| anyhow::anyhow!("{e}"))?;
    let http = reqwest::blocking::Client::builder()
        .tls_backend_preconfigured((*tls).clone())
        .user_agent("pb-xtask-freshness (https://github.com/PaperTobi/FluxerDiscordManagementDashboard)")
        .timeout(Duration::from_secs(30))
        .build()?;
    let gh_token = std::env::var("GITHUB_TOKEN").ok();
    let mut gh_limited = false;
    // Several crates share a repository (tokio, tracing, leptos, burn …): ask once each.
    let mut seen: BTreeMap<(String, String), RepoState> = BTreeMap::new();

    let mut problems = Vec::new();
    println!(
        "{:<28} {:<16} {:<16} {:>8} {:>8}  repository",
        "crate", "locked", "newest", "release", "push"
    );
    for (name, req_set) in &reqs {
        let resp: CrateResponse = http
            .get(format!("https://crates.io/api/v1/crates/{name}"))
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .with_context(|| format!("crates.io: {name}"))?
            .json()?;
        std::thread::sleep(Duration::from_millis(1000)); // crates.io asks for at most one request a second
        let newest = resp.krate.max_stable_version.clone().unwrap_or_default();
        let newest_v: Option<Version> = newest.parse().ok();
        let released = resp
            .versions
            .iter()
            .filter(|v| !v.yanked)
            .filter_map(|v| days_since(&v.created_at))
            .min()
            .unwrap_or(i64::MAX);
        let lock = locked
            .get(name.as_str())
            .map(|vs| vs.iter().map(ToString::to_string).collect::<Vec<_>>().join(","))
            .unwrap_or_default();
        let behind = newest_v.as_ref().is_some_and(|nv| {
            req_set
                .iter()
                .any(|r| VersionReq::parse(r).is_ok_and(|req| !req.matches(nv)))
        });
        let outdated_lock = newest_v.as_ref().is_some_and(|nv| {
            locked.get(name.as_str()).is_some_and(|vs| {
                vs.iter().all(|v| *v < nv)
                    && req_set
                        .iter()
                        .all(|r| VersionReq::parse(r).is_ok_and(|req| req.matches(nv)))
            })
        });
        let repo = resp.krate.repository.clone().unwrap_or_default();
        let gh = github(&repo);
        let state = match (gh.clone().and_then(|k| seen.get(&k).cloned()), gh, gh_limited) {
            (Some(known), _, _) => known,
            (None, Some(_), true) => RepoState::Unknown("GitHub rate limit".into()),
            (None, Some((owner, r)), false) => {
                let mut req = http.get(format!("https://api.github.com/repos/{owner}/{r}"));
                if let Some(t) = &gh_token {
                    req = req.bearer_auth(t);
                }
                match req.send() {
                    Ok(res) if res.status() == reqwest::StatusCode::NOT_FOUND => RepoState::Missing,
                    Ok(res) if res.status().is_success() => match res.json::<Repo>() {
                        Ok(r) if r.archived => RepoState::Archived,
                        Ok(r) => RepoState::Ok {
                            days_since_push: r.pushed_at.as_deref().and_then(days_since).unwrap_or(i64::MAX),
                        },
                        Err(e) => RepoState::Unknown(e.to_string()),
                    },
                    Ok(res) => {
                        if matches!(res.status().as_u16(), 403 | 429) {
                            gh_limited = true;
                        }
                        RepoState::Unknown(format!("GitHub answered {}", res.status()))
                    }
                    Err(e) => RepoState::Unknown(e.to_string()),
                }
            }
            (None, None, _) => RepoState::Unknown("not on GitHub".into()),
        };
        if let Some(k) = github(&repo)
            && !matches!(state, RepoState::Unknown(_))
        {
            seen.insert(k, state.clone());
        }
        let push = match &state {
            RepoState::Ok { days_since_push } => format!("{days_since_push}d"),
            RepoState::Archived => "ARCHIVED".into(),
            RepoState::Missing => "MISSING".into(),
            RepoState::Unknown(_) => "?".into(),
        };
        println!("{name:<28} {lock:<16} {newest:<16} {:>7}d {push:>8}  {repo}", released);
        if let RepoState::Unknown(why) = &state {
            println!("    repository not checked: {why}");
        }
        let mut why = Vec::new();
        if behind {
            why.push(format!(
                "{newest} is out and {} does not allow it",
                req_set.iter().cloned().collect::<Vec<_>>().join(" / ")
            ));
        }
        if outdated_lock {
            why.push(format!(
                "locked {lock}, {newest} is allowed: run cargo update -p {name}"
            ));
        }
        match state {
            RepoState::Archived => why.push("its repository is archived".into()),
            RepoState::Missing => why.push(format!("its repository {repo} does not exist")),
            RepoState::Ok { days_since_push } if days_since_push > STALE_DAYS && released > STALE_DAYS => why.push(
                format!("no release for {released} days and no push for {days_since_push} days"),
            ),
            _ => {}
        }
        if !why.is_empty() {
            match allowed.get(name.as_str()) {
                Some(reason) => println!("    allowed: {} ({reason})", why.join("; ")),
                None => problems.push(format!("{name}: {}", why.join("; "))),
            }
        }
    }
    if gh_limited {
        println!("freshness: GitHub's rate limit was reached; set GITHUB_TOKEN to check every repository");
    }
    if problems.is_empty() {
        println!(
            "freshness: {} direct dependencies, all current and maintained",
            reqs.len()
        );
        Ok(())
    } else {
        for p in &problems {
            eprintln!("freshness: {p}");
        }
        bail!(
            "{} dependency problem(s); fix them or explain them in xtask/freshness.toml",
            problems.len()
        )
    }
}
