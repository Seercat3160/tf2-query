use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use tf2_query::QueryClient;
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use tf2_query::filter::{Filter, FilterBuilder};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .or_else(|_| EnvFilter::try_new("tf2_query=info"))
                .unwrap(),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .compact()
                .with_writer(std::io::stderr),
        )
        .init();

    let should_exit = CancellationToken::new();

    let should_exit_clone = should_exit.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        should_exit_clone.cancel();
    });

    let key = std::env::var("STEAM_API_KEY").context("STEAM_API_KEY must be set!")?;

    let client = QueryClient::new(key).await?;

    let server_filter = filter_from_args(&args);

    info!("getting server list from API...");

    select! {
        filtered_servers = client.serverlist(server_filter, args.get_players) => {
            serde_json::to_writer_pretty(std::io::stdout(), &filtered_servers?)?;
        }
        () = should_exit.cancelled() => {
            warn!("Got signal, cancelling!");
        }
    }

    Ok(())
}

#[derive(clap::Parser)]
#[allow(clippy::struct_excessive_bools)]
struct Args {
    /// Read from JSON file rather than performing an API call for the list of servers
    #[clap(long)]
    from_file: Option<PathBuf>,

    /// Query each server for it's online players
    #[clap(long)]
    get_players: bool,

    /// Filter for only servers with online players
    #[clap(long)]
    has_players: bool,

    /// Filter: exclude MvM servers
    #[clap(long)]
    no_mvm: bool,

    /// Filter: only include Valve servers (may not be 100% accurate)
    #[clap(long)]
    valve: bool,

    /// Filter: only Valve servers with this location, e.g. "syd". Implies `--valve`.
    #[clap(long)]
    valve_location: Option<String>,

    /// Filter: only servers matching this name pattern. '*' is a wildcard. Ignored if  `--valve` or `--valve-location` are specified.
    #[clap(long)]
    filter_name: Option<String>,
}

fn filter_from_args(args: &Args) -> Filter {
    let mut forbidden_tags = vec![];
    let mut min_players: i32 = 0;

    if args.no_mvm {
        forbidden_tags.push("mvm".into());
    }

    if args.has_players {
        min_players = 1;
    }

    FilterBuilder::new()
        .only_valve(args.valve)
        .valve_location(args.valve_location.clone())
        .name_filter_pattern(args.filter_name.clone())
        .forbidden_tags(forbidden_tags)
        .min_players(min_players)
        .build()
}
