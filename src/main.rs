use anyhow::Result;
use clap::{Parser, Subcommand};
use mcp_kali::jobs::Scheduler;
use std::{net::SocketAddr, path::PathBuf};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "Asynchronous Kali tool scheduler and MCP bridge")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the Kali-side HTTP scheduler and job monitor.
    Serve {
        #[arg(long, env = "MCP_KALI_BIND", default_value = "127.0.0.1:5000")]
        bind: SocketAddr,
        #[arg(
            long,
            env = "MCP_KALI_STATE_DIR",
            default_value = "/var/lib/mcp-kali/jobs"
        )]
        state_dir: PathBuf,
        #[arg(long, env = "MCP_KALI_MAX_CONCURRENCY", default_value_t = 2)]
        max_concurrency: usize,
        #[arg(long, env = "MCP_KALI_DEFAULT_TIMEOUT", default_value_t = 1800)]
        default_timeout: u64,
    },
    /// Run the local stdio MCP bridge.
    Mcp {
        #[arg(long, env = "MCP_KALI_SERVER", default_value = "http://127.0.0.1:5000")]
        server: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mcp_kali=info,tower_http=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().command {
        Commands::Serve {
            bind,
            state_dir,
            max_concurrency,
            default_timeout,
        } => {
            let scheduler = Scheduler::open(state_dir, max_concurrency, default_timeout).await?;
            mcp_kali::api::serve(bind, scheduler).await
        }
        Commands::Mcp { server } => mcp_kali::mcp::run(server).await,
    }
}
