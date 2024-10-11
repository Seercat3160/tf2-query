use std::fmt::{Display, Formatter};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::anyhow;
use clap::Parser;
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
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
#[allow(clippy::too_many_lines)]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let should_exit = CancellationToken::new();
    let should_loop_exit = should_exit.clone();

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        should_exit.cancel();
    });

    let reqwest_client = reqwest::ClientBuilder::new()
        .user_agent(format!("TF2Query/{} (#FixTF2)", env!("CARGO_PKG_VERSION")))
        .build()?;

    if let Some(subcommand) = args.subcommand {
        match subcommand {
            Subcommands::SDRQuery { addr } => {
                eprintln!(
                    "fakeip integer representation of server IP {}: {:?}",
                    addr.ip(),
                    fakeip_to_int(addr.ip())
                );

                let players =
                    do_fakeip_players_query(addr.ip(), addr.port(), reqwest_client).await?;

                eprintln!("{players:#?}");

                return Ok(());
            }
            Subcommands::A2SQuery { addr } => {
                let mut client = a2s::A2SClient::new().await?;
                let client = client.app_id(440);

                let server_info = match client.info(addr).await {
                    Ok(server_info) => server_info,
                    Err(e) => {
                        eprintln!("Error getting server info: {e:#}");
                        return Ok(());
                    }
                };

                eprintln!("{server_info:#?}");

                let players = match client.players(addr).await {
                    Ok(players) => players,
                    Err(e) => {
                        eprintln!("Error getting players: {e:#}");
                        return Ok(());
                    }
                };

                eprintln!("{players:#?}");

                return Ok(());
            }
        }
    }

    let response_value: serde_json::Value;

    if let Some(file) = args.from_file {
        eprintln!("reading server list from file: {file:?}");

        let text = tokio::fs::read_to_string(file).await?;

        response_value = serde_json::from_str(&text)?;
    } else {
        eprintln!("getting server list from API...");

        // This could be done better, like some proper structured representation from which the filter string can be constructed
        // See https://github.com/MegaAntiCheat/masterbase/blob/64ada88eff0d398ae229a44db2eeb8a31f00b126/masterbase/steam.py#L51
        let filter_string = if args.valve {
            "appid\\440\\gametype\\valve\\name_match\\Valve Matchmaking Server *\\secure\\1\\linux\\1"
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
                            // arbitrary large number to hopefully get all servers, there doesn't seem to be any pagination.
                            // I've not seen it return more than 10,000 servers at a time even with this higher than that.
                            "limit": 100_000
                        }
                    )
                    .to_string(),
                ),
            ]);

        // print the current time in UTC
        eprintln!("time: {}", chrono::Utc::now());

        let response = req.send().await?;

        let response_text = &response.text().await?;

        response_value = serde_json::from_str(response_text)?;
    }

    let raw_servers: Vec<RawServer> = serde_json::from_value(
        response_value
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

    let fewer_servers: Vec<&Server> = servers
        .iter()
        .filter(|server| {
            if args.has_players {
                server.num_players > 0
            } else {
                true
            }
        })
        .filter(|server| {
            if args.no_mvm {
                !server.tags.contains(&"mvm".to_string())
            } else {
                true
            }
        })
        .filter(|server| {
            // if it has `valve` in the tags, make sure it has valid valve server location data, and if we're filtering for valve servers, make sure it's all good
            if server.tags.contains(&"valve".to_string()) {
                match &server.valve_server_location {
                    Some(valve_server_location) => {
                        // check filters based on this data
                        if args.valve {
                            if let Some(region) = &args.valve_region {
                                if region != &valve_server_location.region {
                                    return false;
                                }
                            }
                            if let Some(cluster) = &args.valve_cluster {
                                if cluster != &valve_server_location.cluster {
                                    return false;
                                }
                            }
                            if let Some(pop) = &args.valve_pop {
                                if pop != &valve_server_location.pop {
                                    return false;
                                }
                            }
                            if let Some(instance) = &args.valve_instance {
                                if instance != &valve_server_location.instance {
                                    return false;
                                }
                            }
                        }
                    }
                    None => {
                        if args.valve {
                            // most likely not actually a valve server, just ignore it
                            return false;
                        }
                    }
                }
            }
            true
        })
        .collect();

    let mut interval = tokio::time::interval(Duration::from_secs(1));

    let mut output_servers: Vec<Server> = Vec::with_capacity(fewer_servers.len());

    eprintln!("{} servers found", fewer_servers.len());

    for (index, server) in fewer_servers.iter().enumerate() {
        let mut server = (**server).clone();

        if should_loop_exit.is_cancelled() {
            break;
        }

        if args.get_players && server.num_players.is_positive() {
            select! {
                () = should_loop_exit.cancelled() => break,
                _ = interval.tick() => {}
            }

            eprintln!(
                "({} of {}) - fetching players for {} ({}:{})...",
                index + 1,
                fewer_servers.len(),
                server.name,
                server.ip,
                server.port
            );

            match server.fetch_players(reqwest_client.clone()).await {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("Error fetching players: {e:#}");
                    // skip this server, but keep going
                    continue;
                }
            }
        } else {
            eprintln!(
                "({} of {}) - not fetching players for {} ({}:{})...",
                index + 1,
                fewer_servers.len(),
                server.name,
                server.ip,
                server.port
            );
        }

        output_servers.push(server);
    }

    serde_json::to_writer_pretty(std::io::stdout(), &output_servers)?;

    Ok(())
}

async fn do_fakeip_players_query(
    ip: IpAddr,
    port: u16,
    reqwest_client: reqwest::Client,
) -> anyhow::Result<Vec<Player>> {
    let request = reqwest_client
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

    let response = request.send().await?;

    let response_text = &response.text().await?;

    let response_value: serde_json::Value = serde_json::from_str(response_text)?;

    let players: Vec<Player>;

    if let Some(res_response) = response_value.get("response").and_then(|x| x.as_object()) {
        if let Some(res_players_data) = res_response.get("players_data").and_then(|x| x.as_object())
        {
            if let Some(res_players) = res_players_data.get("players").and_then(|x| x.as_array()) {
                players = match res_players
                    .iter()
                    .map(|x| {
                        serde_json::from_value(x.clone())
                            .map_err(|e| anyhow!("Error parsing player data: {e:#}"))
                    })
                    .collect::<Result<Vec<_>, _>>()
                {
                    Ok(players) => players,
                    Err(e) => {
                        // print additional data to help debug the issue where score is sometimes u32::MAX rather than an i32
                        eprintln!("Error parsing. Data: {res_players:#?}");
                        return Err(e);
                    }
                };

                Ok(players)
            } else {
                // no players on this server
                Ok(vec![])
            }
        } else {
            Err(anyhow!("response has no 'players_data' key"))
        }
    } else {
        Err(anyhow!("response has no 'response' key"))
    }
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

    /// Filter for only Valve servers
    #[clap(long)]
    valve: bool,

    /// Filter: only Valve servers with the given region. Requires `--valve`.
    #[clap(long)]
    valve_region: Option<ValveMatchmakingRegion>,

    /// Filter: only Valve servers with the given cluster. Requires `--valve`.
    #[clap(long)]
    valve_cluster: Option<i32>,

    /// Filter: only Valve servers with the given PoP. Requires `--valve`.
    #[clap(long)]
    valve_pop: Option<String>,

    /// Filter: only Valve servers with the given instance number. Requires `--valve`.
    #[clap(long)]
    valve_instance: Option<i32>,

    #[clap(subcommand)]
    subcommand: Option<Subcommands>,
}

#[derive(clap::Subcommand)]
enum Subcommands {
    /// Perform a Fake IP query, returning a list of players
    SDRQuery {
        /// Address of the server
        addr: SocketAddr,
    },
    /// Perform an A2S query, returning server info and players.
    A2SQuery {
        /// Address of the server
        addr: SocketAddr,
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

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
/// The server data we care about, with better types.
struct Server {
    ip: IpAddr,
    port: u16,
    name: String,
    region: Region,
    players: Option<Vec<Player>>,
    num_players: i32,
    max_players: i32,
    bots: i32,
    map: String,
    tags: Vec<String>,
    /// Data parsed from the server's name following the format (e.g.) "Valve Matchmaking Server (Sydney srcds1013-syd1 #252)"
    valve_server_location: Option<ValveServerLocation>,
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
                valve_server_location: server.name.clone().parse::<ValveServerLocation>().ok(),
                name: server.name,
                region: server.region.into(),
                players: None,
                num_players: server.players,
                max_players: server.max_players,
                bots: server.bots,
                map: server.map,
                tags: server
                    .gametype
                    .split(',')
                    .map(std::string::ToString::to_string)
                    .collect(),
            })
        } else {
            Err(anyhow!("invalid server address: {input_addr}"))
        }
    }
}

impl Server {
    /// Fetch players for this server, if they haven't already been fetched
    async fn fetch_players(&mut self, reqwest_client: reqwest::Client) -> anyhow::Result<()> {
        if self.players.is_some() {
            return Ok(());
        }

        match self.ip {
            IpAddr::V4(ip) => {
                if ip.is_link_local() {
                    let players =
                        do_fakeip_players_query(self.ip, self.port, reqwest_client).await?;
                    self.players = Some(players);
                    Ok(())
                } else {
                    Err(anyhow!(
                        "We can only query players for servers behind a Fake IP"
                    ))
                }
            }
            IpAddr::V6(_) => Err(anyhow!(
                "IPv6 not supported yet - we can only query players for servers behind a Fake IP"
            )),
        }
    }
}

/// A server region as reported by the Steam API. See https://developer.valvesoftware.com/wiki/Sv_region
#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Deserialize, Clone, Serialize)]
struct Player {
    name: String,
    // I don't remember why I made this an i32 initially.
    // According to https://developer.valvesoftware.com/wiki/Server_queries#A2S_PLAYER,
    // the version of this used by queries directly to a gameserver is a C++ long, which is a signed 32-bit integer.
    // However, as of 2024-10-11 around 3:30 UTC, I've seen it sometimes be 4294967295
    // (2^32 - 1, so the maximum value of an UNSIGNED 32-bit integer) and thus fail to deserialize.
    // I wonder if this is related to casual matchmaking reportedly being broken for some today?
    score: i32,
    time_played: u32,
}

impl Display for Player {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl From<Player> for String {
    fn from(player: Player) -> Self {
        player.name
    }
}

/// Convert an IP address to an integer, ready to be passed to Steam's fake IP API.
fn fakeip_to_int(ip: IpAddr) -> Option<u32> {
    match ip {
        IpAddr::V4(ip) => {
            let ip_parts: [u8; 4] = ip.octets();
            let ip_integer = u32::from(ip_parts[0]) << 24
                | u32::from(ip_parts[1]) << 16
                | u32::from(ip_parts[2]) << 8
                | u32::from(ip_parts[3]);
            Some(ip_integer)
        }
        IpAddr::V6(_) => None,
    }
}

/// Information about an official TF2 server (Casual, Competitive, MvM, etc) determined by parsing it's name.
/// Field names are all guesses as to what the different parts mean.
#[derive(Debug, PartialEq, Eq, Clone, Serialize)]
struct ValveServerLocation {
    region: ValveMatchmakingRegion,
    /// Point-of-Presence.
    /// In the form `xxxn` where `xxx` seems to correspond to `region` and `n` is the datacenter number.
    /// Generally one per region but can be multiple in some cases e.g. LA has lax1 and lax2.
    /// Seems to be similar to those returned by https://api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=440
    /// Note: use `jq '.pops | with_entries({ key: .key, value: .value.desc })'` on that endpoint
    pop: String,
    /// I don't really know what this is.
    /// It doesn't seem to be unique to a region. That is, there can be servers with this the same across multiple regions.
    /// It's the digits after `srcds`
    cluster: i32,
    /// I don't really know what this is either.
    /// It's the digits after `#` at the end.
    instance: i32,
}

impl FromStr for ValveServerLocation {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match VALVE_SERVER_LOCATION_REGEX.captures(s) {
            Some(captures) => Ok(ValveServerLocation {
                region: captures.get(1).unwrap().as_str().parse()?,
                pop: captures.get(3).unwrap().as_str().into(),
                cluster: captures.get(2).unwrap().as_str().parse()?,
                instance: captures.get(4).unwrap().as_str().parse()?,
            }),
            None => Err(anyhow!(
                "couldn't parse server name into ValveServerLocation: {s}"
            )),
        }
    }
}

impl Display for ValveServerLocation {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} srcds{}-{} #{}",
            self.region, self.cluster, self.pop, self.instance
        )
    }
}

lazy_static! {
    static ref VALVE_SERVER_LOCATION_REGEX: Regex =
        regex::Regex::new(r"^Valve Matchmaking Server \(([[:alpha:] ]+) srcds([[:digit:]]+)-([[:lower:]]{3}[[:digit:]]+) #([[:digit:]]+)\)$").expect("regex should be valid");
}

/// The region/city name found in the hostname of official Valve Matchmaking Servers.
/// Non-exhaustive because these are the only ones I've seen be returned but there could be others.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
enum ValveMatchmakingRegion {
    Brazil,
    Chennai,
    Chile,
    Dubai,
    Frankfurt,
    HongKong,
    Johannesburg,
    LA,
    Madrid,
    Mumbai,
    Peru,
    Singapore,
    Stockholm,
    Sydney,
    Tokyo,
    Virginia,
    Washington,
}

impl Display for ValveMatchmakingRegion {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ValveMatchmakingRegion::Brazil => write!(f, "Brazil"),
            ValveMatchmakingRegion::Chennai => write!(f, "Chennai"),
            ValveMatchmakingRegion::Chile => write!(f, "Chile"),
            ValveMatchmakingRegion::Dubai => write!(f, "Dubai"),
            ValveMatchmakingRegion::Frankfurt => write!(f, "Frankfurt"),
            ValveMatchmakingRegion::HongKong => write!(f, "Hong Kong"),
            ValveMatchmakingRegion::Johannesburg => write!(f, "Johannesburg"),
            ValveMatchmakingRegion::LA => write!(f, "LA"),
            ValveMatchmakingRegion::Madrid => write!(f, "Madrid"),
            ValveMatchmakingRegion::Mumbai => write!(f, "Mumbai"),
            ValveMatchmakingRegion::Peru => write!(f, "Peru"),
            ValveMatchmakingRegion::Singapore => write!(f, "Singapore"),
            ValveMatchmakingRegion::Stockholm => write!(f, "Stockholm"),
            ValveMatchmakingRegion::Sydney => write!(f, "Sydney"),
            ValveMatchmakingRegion::Tokyo => write!(f, "Tokyo"),
            ValveMatchmakingRegion::Virginia => write!(f, "Virginia"),
            ValveMatchmakingRegion::Washington => write!(f, "Washington"),
        }
    }
}

impl FromStr for ValveMatchmakingRegion {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Brazil" => Ok(ValveMatchmakingRegion::Brazil),
            "Chennai" => Ok(ValveMatchmakingRegion::Chennai),
            "Chile" => Ok(ValveMatchmakingRegion::Chile),
            "Dubai" => Ok(ValveMatchmakingRegion::Dubai),
            "Frankfurt" => Ok(ValveMatchmakingRegion::Frankfurt),
            "Hong Kong" => Ok(ValveMatchmakingRegion::HongKong),
            "Johannesburg" => Ok(ValveMatchmakingRegion::Johannesburg),
            "LA" => Ok(ValveMatchmakingRegion::LA),
            "Madrid" => Ok(ValveMatchmakingRegion::Madrid),
            "Mumbai" => Ok(ValveMatchmakingRegion::Mumbai),
            "Peru" => Ok(ValveMatchmakingRegion::Peru),
            "Singapore" => Ok(ValveMatchmakingRegion::Singapore),
            "Stockholm" => Ok(ValveMatchmakingRegion::Stockholm),
            "Sydney" => Ok(ValveMatchmakingRegion::Sydney),
            "Tokyo" => Ok(ValveMatchmakingRegion::Tokyo),
            "Virginia" => Ok(ValveMatchmakingRegion::Virginia),
            "Washington" => Ok(ValveMatchmakingRegion::Washington),
            _ => Err(anyhow!("unknown region: {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    #[allow(clippy::unreadable_literal)]
    fn test_fakeip_to_int() {
        assert_eq!(
            fakeip_to_int(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
            Some(0b01111111_00000000_00000000_00000001)
        );
        assert_eq!(
            fakeip_to_int(IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1))),
            None
        );
    }

    #[test]
    fn test_region_from_str() {
        assert_eq!(
            ValveMatchmakingRegion::from_str("Hong Kong").unwrap(),
            ValveMatchmakingRegion::HongKong
        );
    }

    #[test]
    fn test_region_from_str_err() {
        assert!(ValveMatchmakingRegion::from_str("foo").is_err());
    }

    #[test]
    fn test_region_into_str() {
        assert_eq!(ValveMatchmakingRegion::HongKong.to_string(), "Hong Kong");
    }

    #[test]
    fn test_server_location_from_str() {
        assert_eq!(
            ValveServerLocation::from_str("Valve Matchmaking Server (Sydney srcds1013-syd1 #327)")
                .unwrap(),
            ValveServerLocation {
                region: ValveMatchmakingRegion::Sydney,
                pop: "syd1".into(),
                cluster: 1013,
                instance: 327,
            }
        );
    }
}
