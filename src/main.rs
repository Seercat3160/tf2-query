use std::net::{SocketAddr, SocketAddrV4};
use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use tokio::select;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::api::{ApiClient, Server, fakeip_to_int};
use crate::filter::Filter;

mod api;
mod filter;

#[tokio::main]
#[allow(clippy::too_many_lines)]
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
    let should_loop_exit = should_exit.clone();

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        should_exit.cancel();
    });

    let key = std::env::var("STEAM_API_KEY").context("STEAM_API_KEY must be set!")?;
    let client = ApiClient::new(key)?;

    if let Some(subcommand) = args.subcommand {
        match subcommand {
            Subcommands::SDRQuery { addr } => {
                info!(
                    "integer representation of server IP {}: {:?}",
                    addr.ip(),
                    fakeip_to_int(*addr.ip())
                );

                let players = client.sdr_playerlist_query(addr).await?;

                info!("{players:#?}");

                return Ok(());
            }
            Subcommands::A2SQuery { addr } => {
                let mut client = a2s::A2SClient::new().await?;
                let client = client.app_id(440);

                let server_info = match client.info(addr).await {
                    Ok(server_info) => server_info,
                    Err(e) => {
                        anyhow::bail!("Error getting server info: {e:#}");
                    }
                };

                info!("{server_info:#?}");

                let players = match client.players(addr).await {
                    Ok(players) => players,
                    Err(e) => {
                        anyhow::bail!("Error getting players: {e:#}");
                    }
                };

                info!("{players:#?}");

                return Ok(());
            }
        }
    }

    let server_filter = Filter::new(&args);

    info!("getting server list from API...");

    let serverlist_unfiltered = client
        .serverlist(server_filter.build_api_filter_expression())
        .await?;

    let mut servers = Vec::with_capacity(serverlist_unfiltered.len());

    // parse the data to turn each RawServer into a Server
    for server in serverlist_unfiltered {
        // keep moving on if there's an error with any particular server
        let server: Server = match server.try_into() {
            Ok(server) => server,
            Err(e) => {
                error!("error parsing server data: {e:#}");
                continue;
            }
        };

        servers.push(server);
    }

    let num_unfiltered_servers = servers.len();

    let mut filtered_servers: Vec<Server> = servers
        .into_iter()
        .filter(|s| server_filter.filter_server(s))
        .collect();

    let num_filtered_servers = filtered_servers.len();

    let mut interval = tokio::time::interval(Duration::from_secs(1));

    info!(
        "{num_filtered_servers} servers found after additional filtering (out of {num_unfiltered_servers} returned by the API)"
    );

    // fetch players
    for (index, server) in filtered_servers.iter_mut().enumerate() {
        if should_loop_exit.is_cancelled() {
            break;
        }

        if args.get_players && server.num_players.is_positive() {
            select! {
                () = should_loop_exit.cancelled() => break,
                _ = interval.tick() => {}
            }

            info!(
                "({} of {}) - fetching players for {} ({}:{})...",
                index + 1,
                num_filtered_servers,
                server.name,
                server.ip,
                server.port
            );

            match server.fetch_players(&client).await {
                Ok(()) => {}
                Err(e) => {
                    // This server will still be output but won't have player data
                    error!("Error fetching players: {e:#}");
                }
            }
        } else {
            info!(
                "({} of {}) - not fetching players for {} ({}:{})...",
                index + 1,
                num_filtered_servers,
                server.name,
                server.ip,
                server.port
            );
        }
    }

    serde_json::to_writer_pretty(std::io::stdout(), &filtered_servers)?;

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

    #[clap(subcommand)]
    subcommand: Option<Subcommands>,
}

#[derive(clap::Subcommand)]
enum Subcommands {
    /// Perform a Fake IP query, returning a list of players
    SDRQuery {
        /// Address of the server
        addr: SocketAddrV4,
    },
    /// Perform an A2S query, returning server info and players.
    A2SQuery {
        /// Address of the server
        addr: SocketAddr,
    },
}
