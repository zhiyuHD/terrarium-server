use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::{net::SocketAddr, path::PathBuf};
use terrarium_server::{
    http::{App, router},
    store::{LegacySnapshot, Store},
};
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Check a stopped legacy snapshot; write only with --apply.
    ImportLegacy {
        #[arg(long)]
        from: PathBuf,
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        apply: bool,
    },
    ExportLegacy {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Backup {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Check {
        #[arg(long)]
        database: PathBuf,
    },
    /// Read-only migration preview. Public write endpoints are not enabled.
    Serve {
        #[arg(
            long,
            env = "CORTICO_TERRARIUM_DATABASE",
            default_value = "data/terrarium.sqlite"
        )]
        database: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8790")]
        listen: SocketAddr,
    },
}
#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::ImportLegacy {
            from,
            database,
            apply,
        } => {
            let snapshot = LegacySnapshot::read(&from)?;
            let (boxes, credentials, documents) = snapshot.counts();
            if apply {
                Store::open(database)?.import(snapshot)?;
            }
            println!(
                "{}: {boxes} boxes, {credentials} credentials, {documents} documents",
                if apply { "imported" } else { "dry run" }
            );
        }
        Command::ExportLegacy { database, output } => {
            Store::existing(database)?.export(&output)?;
            println!("exported");
        }
        Command::Backup { database, output } => {
            Store::existing(database)?.backup(&output)?;
            println!("backed up");
        }
        Command::Check { database } => {
            Store::existing(database)?.check()?;
            println!("ok");
        }
        Command::Serve { database, listen } => {
            anyhow::ensure!(
                listen.ip().is_loopback(),
                "Migration preview must bind to loopback"
            );
            let store = Store::existing(database)?;
            store.check()?;
            let listener = tokio::net::TcpListener::bind(listen)
                .await
                .context("Listen failed")?;
            println!("Read-only Rust preview on {}", listener.local_addr()?);
            axum::serve(
                listener,
                router(App {
                    store,
                    release_id: std::env::var("CORTICO_TERRARIUM_RELEASE_ID").ok(),
                }),
            )
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?;
        }
    }
    Ok(())
}
