use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::{net::SocketAddr, path::PathBuf};
use terrarium_server::{
    admin::{self, AdminNetwork},
    http::{App, router},
    ingestion::Ingestion,
    store::{LegacySnapshot, Store},
};
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
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
    BackupForum {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Check {
        #[arg(long)]
        database: PathBuf,
    },
    /// Print the stored administrator credential; creation is explicit.
    AdminToken {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        create: bool,
    },
    /// Loopback migration preview; writes require explicit opt-in.
    Serve {
        #[arg(
            long,
            env = "CORTICO_TERRARIUM_DATABASE",
            default_value = "data/terrarium.sqlite"
        )]
        database: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8790")]
        listen: SocketAddr,
        #[arg(long)]
        enable_writes: bool,
        #[arg(long, env = "CORTICO_TERRARIUM_ADMIN_LISTEN")]
        admin_listen: Option<SocketAddr>,
        #[arg(
            long,
            requires = "admin_listen",
            env = "CORTICO_TERRARIUM_ADMIN_ALLOWED_CIDR"
        )]
        admin_cidr: Option<String>,
    },
}
fn main() -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(8)
        .enable_all()
        .build()?
        .block_on(run())
}
async fn run() -> Result<()> {
    let command = Cli::parse()
        .command
        .map(Ok)
        .unwrap_or_else(managed_command)?;
    match command {
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
        Command::BackupForum { database, output } => {
            terrarium_server::forum::backup(&Store::existing(database)?, &output)?;
            println!("backed up forum");
        }
        Command::Backup { database, output } => {
            Store::existing(database)?.backup(&output)?;
            println!("backed up");
        }
        Command::Check { database } => {
            Store::existing(database)?.check()?;
            println!("ok");
        }
        Command::AdminToken { database, create } => {
            println!("{}", Store::existing(database)?.admin_token(create)?);
        }
        Command::Serve {
            database,
            listen,
            enable_writes,
            admin_listen,
            admin_cidr,
        } => {
            anyhow::ensure!(
                listen.ip().is_loopback(),
                "Migration preview must bind to loopback"
            );
            let cidr = admin_listen
                .map(|listen| admin_cidr.unwrap_or_else(|| format!("{}/32", listen.ip())));
            if let Some(bind) = admin_listen {
                AdminNetwork::new(bind, cidr.as_deref().unwrap())?;
            }
            let store = Store::existing(database)?;
            store.check()?;
            let ingestion = enable_writes.then(|| Ingestion::new(store.clone()));
            if let Some(ingestion) = &ingestion {
                ingestion.ready().await?;
            }
            let listener = tokio::net::TcpListener::bind(listen)
                .await
                .context("Listen failed")?;
            let admin_listener = if let Some(listen) = admin_listen {
                Some(
                    tokio::net::TcpListener::bind(listen)
                        .await
                        .context("Admin listen failed")?,
                )
            } else {
                None
            };
            if admin_listener.is_some() {
                store.admin_token(enable_writes)?;
            }
            let digest_store = store.clone();
            let app = App {
                store,
                ingestion,
                release_id: std::env::var("CORTICO_TERRARIUM_RELEASE_ID").ok(),
                release_root: std::env::var_os("CORTICO_TERRARIUM_DEPLOY_ROOT").map(PathBuf::from),
            };
            let shutdown_request = shutdown_signal();
            let private = if let Some(listener) = admin_listener {
                let bind = listener.local_addr()?;
                let network = AdminNetwork::new(bind, cidr.as_deref().unwrap())?;
                println!("Private admin on {bind}");
                Some((listener, admin::router(app.clone(), network)))
            } else {
                None
            };
            println!(
                "Rust preview on {} (writes: {})",
                listener.local_addr()?,
                enable_writes
            );
            let (shutdown, receiver) = tokio::sync::watch::channel(false);
            let public_stop = receiver.clone();
            let digest_stop = receiver.clone();
            let public = async move {
                axum::serve(listener, router(app))
                    .with_graceful_shutdown(wait_for_shutdown(public_stop))
                    .await
            };
            let admin = async move {
                if let Some((listener, router)) = private {
                    axum::serve(
                        listener,
                        router.into_make_service_with_connect_info::<SocketAddr>(),
                    )
                    .with_graceful_shutdown(wait_for_shutdown(receiver))
                    .await?;
                } else {
                    wait_for_shutdown(receiver).await;
                }
                Ok::<(), std::io::Error>(())
            };
            let stop = async move {
                shutdown_request.await;
                let _ = shutdown.send(true);
                Ok::<(), std::io::Error>(())
            };
            tokio::try_join!(
                public,
                admin,
                stop,
                terrarium_server::forum_digest::run(digest_store, enable_writes, digest_stop)
            )?;
        }
    }
    Ok(())
}

fn managed_command() -> Result<Command> {
    let root = PathBuf::from(
        std::env::var("CORTICO_TERRARIUM_DEPLOY_ROOT")
            .context("Supply a subcommand or run through the release controller")?,
    );
    let id = std::env::var("CORTICO_TERRARIUM_RELEASE_ID")
        .context("Managed startup requires a release ID")?;
    anyhow::ensure!(
        root.is_absolute() && terrarium_server::assets::valid_release_id(&id),
        "Invalid managed deployment identity"
    );
    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "8787".into())
        .parse::<u16>()?;
    let admin_listen = std::env::var("CORTICO_TERRARIUM_ADMIN_HOST")
        .ok()
        .map(|host| {
            let port =
                std::env::var("CORTICO_TERRARIUM_ADMIN_PORT").unwrap_or_else(|_| "8788".into());
            format!("{host}:{port}").parse::<SocketAddr>()
        })
        .transpose()?;
    Ok(Command::Serve {
        database: std::env::var_os("CORTICO_TERRARIUM_DATABASE")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("data/terrarium.sqlite")),
        listen: SocketAddr::from(([127, 0, 0, 1], port)),
        enable_writes: true,
        admin_listen,
        admin_cidr: std::env::var("CORTICO_TERRARIUM_ADMIN_ALLOWED_CIDR").ok(),
    })
}

fn shutdown_signal() -> impl std::future::Future<Output = ()> {
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("SIGTERM handler");
    async move {
        #[cfg(unix)]
        tokio::select! { _=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{} }
        #[cfg(not(unix))]
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn wait_for_shutdown(mut receiver: tokio::sync::watch::Receiver<bool>) {
    if !*receiver.borrow() {
        let _ = receiver.changed().await;
    }
}
