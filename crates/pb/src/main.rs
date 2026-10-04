//! `pb`: Profanity Watch, a voice moderation bot for Fluxer. `pb run` starts it; the other commands help the
//! operator.

mod config;
mod run;
mod secrets;
mod tools;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "pb", version, about = "Profanity Watch: a voice moderation bot for Fluxer")]
struct Cli {
    /// The data directory.
    #[arg(long, env = "PB_DATA", default_value = "/data", global = true)]
    data: PathBuf,
    /// The configuration file (default: <data>/config.toml; it may be missing).
    #[arg(long, env = "PB_CONFIG", global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Runs the bot.
    Run,
    /// Prints the setup code (only while setup is unfinished).
    SetupCode,
    /// Lost access to the web UI? Makes the next start ask for a new setup code (logins end; the token, the client
    /// secret, the settings and all data stay).
    ResetSetup,
    /// Downloads the pinned model weights and voices (resumable, each file checked against its SHA-256).
    FetchWeights {
        /// Where to put them (default: the configured weights directory).
        #[arg(long)]
        dest: Option<PathBuf>,
        /// Only check what is there.
        #[arg(long)]
        check: bool,
    },
    /// Imports the old (Python) bot's data directory into this empty one.
    Import {
        #[arg(long)]
        from: PathBuf,
    },
    /// The event log and the search index.
    Store {
        #[command(subcommand)]
        cmd: StoreCmd,
    },
    /// The settings files.
    Settings {
        #[command(subcommand)]
        cmd: SettingsCmd,
    },
    /// Checks the installation (configuration, CPU, weights, files, data directory) without starting the bot.
    Doctor,
    /// Asks a running bot whether it serves (for container health checks); exit 0 = yes.
    Health {
        /// Where the bot listens (default: the configured web address, on this machine).
        #[arg(long)]
        addr: Option<std::net::SocketAddr>,
    },
}

#[derive(Subcommand, Debug)]
enum StoreCmd {
    /// Checks every line of the event log against its hash chain.
    Verify,
    /// Builds the search index again from the event log (stop the bot first).
    RebuildIndex,
}

#[derive(Subcommand, Debug)]
enum SettingsCmd {
    /// Reads every settings file and reports problems.
    Check,
    /// Prints every setting as Markdown.
    Docs {
        #[arg(long, default_value = "en")]
        lang: String,
    },
}

/// How the process ends (the code tells a supervisor whether restarting helps).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Ok = 0,
    /// A bug or an unexpected failure.
    Internal = 1,
    /// Another bot process holds the data directory.
    LockHeld = 3,
    /// A configuration problem that a restart does not fix (bad config file, missing weights, unusable CPU).
    Config = 78,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let file = cli.config.clone().unwrap_or_else(|| cli.data.join("config.toml"));
    let exit = match cli.cmd {
        Cmd::Run => run::main(&cli.data, &file),
        Cmd::SetupCode => match std::fs::read_to_string(cli.data.join("setup-code")) {
            Ok(code) => {
                println!("{}", code.trim());
                Exit::Ok
            }
            Err(_) => {
                eprintln!("No setup code: setup is finished (or the bot has not started yet).");
                Exit::Config
            }
        },
        Cmd::Health { addr } => match config::load(&file) {
            Ok(c) => health(addr.unwrap_or_else(|| local(c.web.bind)), c.web.tls.as_ref()),
            Err(e) => {
                eprintln!("pb: {e}");
                Exit::Config
            }
        },
        Cmd::ResetSetup => tools::reset_setup(&cli.data),
        Cmd::FetchWeights { dest, check } => tools::fetch_weights(&file, dest.as_deref(), check),
        Cmd::Import { from } => tools::import(&cli.data, &file, &from),
        Cmd::Store { cmd: StoreCmd::Verify } => tools::store_verify(&cli.data),
        Cmd::Store {
            cmd: StoreCmd::RebuildIndex,
        } => tools::store_rebuild_index(&cli.data),
        Cmd::Settings {
            cmd: SettingsCmd::Check,
        } => tools::settings_check(&cli.data),
        Cmd::Settings {
            cmd: SettingsCmd::Docs { lang },
        } => tools::settings_docs(&lang),
        Cmd::Doctor => tools::doctor(&cli.data, &file),
    };
    ExitCode::from(exit as u8)
}

/// `GET /healthz` over plain HTTP on the local port.
/// The address to reach a listener bound to `bind` from this machine.
fn local(bind: std::net::SocketAddr) -> std::net::SocketAddr {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    match bind.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => (Ipv4Addr::LOCALHOST, bind.port()).into(),
        IpAddr::V6(ip) if ip.is_unspecified() => (Ipv6Addr::LOCALHOST, bind.port()).into(),
        _ => bind,
    }
}

fn health(addr: std::net::SocketAddr, tls: Option<&config::Tls>) -> Exit {
    let ask = || -> Result<String, String> {
        let mut tcp = std::net::TcpStream::connect(addr).map_err(|e| e.to_string())?;
        tcp.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        let request = "GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n";
        let mut out = String::new();
        match tls {
            None => {
                write!(tcp, "{request}").map_err(|e| e.to_string())?;
                tcp.read_to_string(&mut out).map_err(|e| e.to_string())?;
            }
            // Over HTTPS, trusting exactly the configured certificate (whatever names it carries).
            Some(t) => {
                let name = rustls::pki_types::ServerName::try_from("localhost").map_err(|e| e.to_string())?;
                let conn = rustls::ClientConnection::new(t.pinned_client()?, name).map_err(|e| e.to_string())?;
                let mut s = rustls::StreamOwned::new(conn, tcp);
                write!(s, "{request}").map_err(|e| e.to_string())?;
                // The server may close without a TLS close notification; what arrived counts.
                if let Err(e) = s.read_to_string(&mut out)
                    && out.is_empty()
                {
                    return Err(e.to_string());
                }
            }
        }
        Ok(out)
    };
    match ask() {
        Ok(r) if r.starts_with("HTTP/1.0 200") || r.starts_with("HTTP/1.1 200") => {
            println!("{}", r.split("\r\n\r\n").nth(1).unwrap_or("ok").trim());
            Exit::Ok
        }
        Ok(r) => {
            eprintln!("{}", r.lines().next().unwrap_or("no answer"));
            Exit::Internal
        }
        Err(e) => {
            eprintln!("{addr}: {e}");
            Exit::Internal
        }
    }
}
