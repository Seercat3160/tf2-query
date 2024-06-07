use std::net::IpAddr;
use std::path::PathBuf;

use anyhow::anyhow;
use clap::Parser;
use lazy_static::lazy_static;
use serde::Deserialize;
use serde_json::json;
use tokio::select;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use url::Url;

const STEAM_API_BASE: &str = "https://api.steampowered.com";

lazy_static! {
    static ref STEAM_API_URL: Url =
        Url::parse(STEAM_API_BASE).expect("STEAM_API_BASE must be a valid URL");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let should_exit = CancellationToken::new();
    let should_loop_exit = should_exit.clone();

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        should_exit.cancel();
    });

    let reqwest_client = reqwest::ClientBuilder::new()
        .user_agent("TF-Data-Tracker/0.1.0")
        .build()?;

    if let Some(subcommand) = args.subcommand {
        match subcommand {
            Subcommands::Query { ip, port } => {
                eprintln!("fakeip: {:?}", fakeip_to_int(ip));

                let players = do_fakeip_players_query(ip, port, reqwest_client).await?;

                eprintln!(
                    "players: {:?}",
                    players.iter().map(|x| { &x.name }).collect::<Vec<_>>()
                );

                return Ok(());
            }
        }
    }

    let response: serde_json::Value;

    if let Some(file) = args.from_file {
        eprintln!("reading server list from file: {file:?}");

        let text = tokio::fs::read_to_string(file).await?;

        response = serde_json::from_str(&text)?;
    } else {
        eprintln!("getting server list from API...");

        // This could be done better, like some proper structured representation from which the filter string can be constructed
        // See https://github.com/MegaAntiCheat/masterbase/blob/64ada88eff0d398ae229a44db2eeb8a31f00b126/masterbase/steam.py#L51
        let filter_string = if args.valve {
            "appid\\440\\gametype\\valve"
        } else {
            "appid\\440\\"
        };

        let req = reqwest_client
            .get(STEAM_API_URL.join("/IGameServersService/GetServerList/v1")?)
            .query(&[
                (
                    "key",
                    std::env::var("STEAM_API_KEY").expect("STEAM_API_KEY must be set"),
                ),
                (
                    "input_json",
                    json!(
                        {
                            "filter": filter_string,
                            "limit": 100000 // arbitrary large number to get all servers, there doesn't seem to be any pagination
                        }
                    )
                    .to_string(),
                ),
            ]);

        // print the current time in UTC
        eprintln!("time: {}", chrono::Utc::now());

        let res = req.send().await?;

        // let status = res.status();
        // eprintln!("status: {status:#}");

        // let headers = res.headers();
        // eprintln!("headers: {headers:#?}");

        let text = &res.text().await?;

        response = serde_json::from_str(text)?;
    }

    let raw_servers: Vec<RawServer> = serde_json::from_value(
        response
            .get("response")
            .ok_or(anyhow!("response has no 'response' key"))?
            .get("servers")
            .ok_or(anyhow!("response has no 'servers' key"))?
            .clone(),
    )?;

    let mut servers = Vec::with_capacity(raw_servers.len());

    for raw_server in raw_servers {
        // keep moving on if there's an error with any particular server
        let server: Server = match raw_server.try_into() {
            Ok(server) => server,
            Err(e) => {
                eprintln!("error parsing server data: {e:#}");
                continue;
            }
        };

        servers.push(server);
    }

    // let fewer_servers = servers.iter().filter(|server| {
    //     server.tags.contains(&"valve".to_string()) // TODO: use the filter options of the Steam API to do these things
    //         && server.players > 0
    //         && server.name.contains("Sydney") // this could probably be improved, and is just a quick hack for testing
    // });

    let fewer_servers = servers.iter().filter(|server| server.players > 0);

    let mut interval = tokio::time::interval(Duration::from_secs(5));

    for server in fewer_servers {
        println!("{server:#?}");

        if should_loop_exit.is_cancelled() {
            break;
        }

        if args.get_players {
            select! {
                _ = should_loop_exit.cancelled() => break,
                _ = interval.tick() => {}
            }

            // print the current time in UTC
            eprintln!("time: {}", chrono::Utc::now());

            let players =
                do_fakeip_players_query(server.ip, server.port, reqwest_client.clone()).await?;

            eprintln!(
                "players: {:?}",
                players.iter().map(|x| { &x.name }).collect::<Vec<_>>()
            );
        }
    }

    Ok(())
}

async fn do_fakeip_players_query(
    ip: IpAddr,
    port: u16,
    reqwest_client: reqwest::Client,
) -> anyhow::Result<Vec<Player>> {
    let req = reqwest_client
        .get(STEAM_API_URL.join("/IGameServersService/QueryByFakeIP/v1")?)
        .query(&[
            (
                "key",
                std::env::var("STEAM_API_KEY").expect("STEAM_API_KEY must be set"),
            ),
            (
                "input_json",
                json!(
                    {
                        "fake_ip": fakeip_to_int(ip).ok_or(anyhow!("FakeIP must be IPv4"))?, // I don't think actual Valve FakeIPs can be IPv6
                        "fake_port": port,
                        "app_id": 440,
                        "query_type": 2
                    }
                )
                .to_string(),
            ),
        ]);

    let res = req.send().await?;

    let text = &res.text().await?;

    let response: serde_json::Value = serde_json::from_str(text)?;

    let players: Vec<Player> = serde_json::from_value(
        response
            .get("response")
            .ok_or(anyhow!("response has no 'response' key"))?
            .get("players_data")
            .ok_or(anyhow!("response has no 'players_data' key"))?
            .get("players")
            .ok_or(anyhow!("response has no 'players' key"))?
            .clone(),
    )?;

    Ok(players)
}

#[derive(clap::Parser)]
struct Args {
    /// Read from JSON file rather than performing an API call for the list of servers
    #[clap(long)]
    from_file: Option<PathBuf>,

    /// Query each server for it's online players
    #[clap(long)]
    get_players: bool,

    /// Filter for only Valve servers
    #[clap(long)]
    valve: bool,

    #[clap(subcommand)]
    subcommand: Option<Subcommands>,
}

#[derive(clap::Subcommand)]
enum Subcommands {
    /// Perform a Fake IP query
    Query {
        /// The IP to query
        ip: IpAddr,
        /// The port
        port: u16,
    },
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
/// A server as returned by IGameServersService/GetServerList/v1. Typed, but just as much as the JSON, so mostly strings. Also, more fields than we need.
struct RawServer {
    addr: String,
    gameport: u16,
    steamid: String,
    name: String,
    appid: i32,
    gamedir: String,
    version: String,
    product: String,
    region: i32,
    players: i32,
    max_players: i32,
    bots: i32,
    map: String,
    secure: bool,
    dedicated: bool,
    os: String,
    #[serde(default)]
    gametype: String,
}

#[derive(Debug)]
#[allow(dead_code)]
/// The server data we care about, with better types.
struct Server {
    ip: IpAddr,
    port: u16,
    name: String,
    region: Region,
    players: i32,
    max_players: i32,
    bots: i32,
    map: String,
    tags: Vec<String>,
}

impl TryFrom<RawServer> for Server {
    type Error = anyhow::Error;

    fn try_from(server: RawServer) -> Result<Self, Self::Error> {
        let input_addr = server.addr;

        let ip_without_port = input_addr.rfind(':').map(|i| &input_addr[..i]);

        if let Some(ip_without_port) = ip_without_port {
            Ok(Server {
                // convert from form `127.0.0.1:8080` to `127.0.0.1` by getting part before the last colon and parsing
                ip: ip_without_port.parse()?,
                port: server.gameport,
                name: server.name,
                region: server.region.into(),
                players: server.players,
                max_players: server.max_players,
                bots: server.bots,
                map: server.map,
                tags: server.gametype.split(',').map(|x| x.to_string()).collect(),
            })
        } else {
            Err(anyhow!("invalid server address: {input_addr}"))
        }
    }
}

/// A server region as reported by the Steam API. See https://developer.valvesoftware.com/wiki/Sv_region
#[derive(Debug)]
enum Region {
    World,
    USEast,
    USWest,
    SouthAmerica,
    Europe,
    Asia,
    Australia,
    MiddleEast,
    Africa,
    #[allow(dead_code)]
    Other(i32),
}

impl From<i32> for Region {
    fn from(region: i32) -> Self {
        match region {
            255 => Region::World,
            0 => Region::USEast,
            1 => Region::USWest,
            2 => Region::SouthAmerica,
            3 => Region::Europe,
            4 => Region::Asia,
            5 => Region::Australia,
            6 => Region::MiddleEast,
            7 => Region::Africa,
            _ => Region::Other(region),
        }
    }
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Player {
    name: String,
    score: i32,
    time_played: u32,
}

/// Convert an IP address to an integer, ready to be passed to Steam's fake IP API.
fn fakeip_to_int(ip: IpAddr) -> Option<u32> {
    match ip {
        IpAddr::V4(ip) => {
            let ip_parts: [u8; 4] = ip.octets();
            let ip_integer = (ip_parts[0] as u32) << 24
                | (ip_parts[1] as u32) << 16
                | (ip_parts[2] as u32) << 8
                | (ip_parts[3] as u32);
            Some(ip_integer)
        }
        IpAddr::V6(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn test_fakeip_to_int() {
        assert_eq!(
            fakeip_to_int(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
            Some(2130706433)
        );
        assert_eq!(
            fakeip_to_int(IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1))),
            None
        );
    }
}
