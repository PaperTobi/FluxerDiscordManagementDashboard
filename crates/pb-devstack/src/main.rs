//! `pb-devstack`: the dev stack as a program. `--ready` prepares a finished setup; without it the setup wizard runs and
//! the values to enter are printed.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use pb_devstack::{Opts, Stack};

#[derive(Parser, Debug)]
struct Args {
    /// The data directory to prepare for `pb run`.
    #[arg(long)]
    data: PathBuf,
    /// Where the fake Fluxer listens (reachable from the browser for logins).
    #[arg(long, default_value = "0.0.0.0:8081")]
    fluxer: SocketAddr,
    /// Where the bot's web UI will listen.
    #[arg(long, default_value = "0.0.0.0:8790")]
    web: SocketAddr,
    /// The address the bot's web UI will have (for the login redirect).
    #[arg(long, default_value = "http://localhost:8790")]
    ui: String,
    /// The model weights (`pb run` reads them).
    #[arg(long, env = "PB_WEIGHTS", default_value = "target/weights")]
    weights: PathBuf,
    /// Prepare a finished setup instead of leaving it to the wizard.
    #[arg(long)]
    ready: bool,
    /// Nobody talks in the call.
    #[arg(long)]
    quiet: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let a = Args::parse();
    let opts = Opts {
        data: a.data,
        fluxer: a.fluxer,
        web: a.web,
        ui: a.ui,
        weights: a.weights,
        ready: a.ready,
        talk: !a.quiet,
    };
    let stack = Stack::start(&opts).await.context("starting the dev stack")?;
    let cfg = stack.fake.config();
    println!();
    println!(
        "fake Fluxer: {}  (logins approve as \"The Owner\", user {})",
        stack.fake.url(),
        cfg.owner_id
    );
    if !opts.ready {
        println!(
            "setup wizard values: instance {}  bot token {}  client secret {}",
            stack.fake.url(),
            cfg.token,
            cfg.client_secret
        );
    }
    println!("start the bot:  PB_DATA={} cargo run -p pb -- run", opts.data.display());
    println!();
    tokio::signal::ctrl_c().await?;
    Ok(())
}
