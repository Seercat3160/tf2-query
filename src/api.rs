//! Code for dealing with the Steam API

use std::{
    fmt::{Display, Formatter},
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    str::FromStr,
    sync::LazyLock,
};

use anyhow::{Context, anyhow};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, warn};
use url::Url;

static STEAM_API_URL: LazyLock<Url> = std::sync::LazyLock::new(|| {
    Url::parse("https://api.steampowered.com").expect("Hardcoded URL should be valid")
});

pub(crate) struct ApiClient {
    client: reqwest::Client,
    key: String,
}

impl ApiClient {
    pub(crate) fn new(key: String) -> anyhow::Result<Self> {
        let client = reqwest::ClientBuilder::new()
            .user_agent(format!("TF2Query/{}", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Self { client, key })
    }
}

impl ApiClient {
    /// Use `IGameServersService/QueryByFakeIP/v1` to get the player list for a server behind Steam Datagram Relay
    pub(crate) async fn sdr_playerlist_query(
        &self,
        addr: SocketAddrV4,
    ) -> anyhow::Result<Vec<Player>> {
        let fakeip_int = fakeip_to_int(*addr.ip());

        let request = self
            .client
            .get(STEAM_API_URL.join("/IGameServersService/QueryByFakeIP/v1")?)
            .query(&[
                ("key", self.key.clone()),
                (
                    "input_json",
                    json!(
                        {
                            "fake_ip": fakeip_int,
                            "fake_port": addr.port(),
                            "app_id": 440,
                            "query_type": 2 // playerlist query - API equivalent of https://developer.valvesoftware.com/wiki/Server_queries#A2S_PLAYER
                        }
                    )
                    .to_string(),
                ),
            ]);

        let response: serde_json::Value = request
            .send()
            .await
            .context("SDR playerlist query failed")?
            .json()
            .await
            .context("Could not deserialize JSON in SDR playerlist response")?;

        let players: Vec<Player> = match response.pointer("/response/players_data/players") {
            Some(val) => serde_json::from_value(val.clone())?,
            None => vec![],
        };

        Ok(players)
    }

    /// Use `IGameServersService/GetServerList/v1` to get the serverlist using a provided filter expression. There is no additional filtering done beyond what the API returns.
    pub(crate) async fn serverlist(&self, filter: String) -> anyhow::Result<Vec<RawServer>> {
        let req = self
            .client
            .get(STEAM_API_URL.join("/IGameServersService/GetServerList/v1")?)
            .query(&[
                ("key", self.key.clone()),
                (
                    "input_json",
                    json!(
                        {
                            "filter": filter,
                            // There doesn't seem to be any pagination, and I've not seen more than 10,000 servers returned at a time despite this.
                            "limit": 100_000
                        }
                    )
                    .to_string(),
                ),
            ]);

        let req = req.build()?;
        debug!("requesting: {}", req.url());
        debug!("filter expression: {filter}");

        let response: serde_json::Value = self.client.execute(req).await?.json().await?;

        let servers_untyped = response
            .pointer("/response/servers")
            .cloned()
            .unwrap_or_else(|| json!([]))
            .as_array()
            .cloned()
            .unwrap_or_default();

        let mut servers: Vec<RawServer> = Vec::with_capacity(servers_untyped.len());

        for val in servers_untyped {
            match serde_json::from_value(val.clone()) {
                Ok(server) => servers.push(server),
                Err(e) => warn!(
                    "could not deserialize JSON from server data {}: {e}",
                    serde_json::to_string_pretty(&val).unwrap_or("{}".into())
                ),
            }
        }

        Ok(servers)
    }
}

/// Convert an IP address to an integer, ready to be passed to Steam's API.
pub(crate) fn fakeip_to_int(ip: Ipv4Addr) -> u32 {
    let ip_parts: [u8; 4] = ip.octets();

    u32::from(ip_parts[0]) << 24
        | u32::from(ip_parts[1]) << 16
        | u32::from(ip_parts[2]) << 8
        | u32::from(ip_parts[3])
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
/// A server as returned by IGameServersService/GetServerList/v1. Typed, but just as much as the JSON, so mostly strings. Also, more fields than we need.
pub(crate) struct RawServer {
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
pub(crate) struct Server {
    pub(crate) ip: IpAddr,
    pub(crate) port: u16,
    pub(crate) name: String,
    region: Region,
    players: Option<Vec<Player>>,
    pub(crate) num_players: i32,
    max_players: i32,
    bots: i32,
    map: String,
    pub(crate) tags: Vec<String>,
    pub(crate) valve_location: Option<ValveServerLocation>,
}

impl TryFrom<RawServer> for Server {
    type Error = anyhow::Error;

    fn try_from(server: RawServer) -> Result<Self, Self::Error> {
        Ok(Server {
            ip: SocketAddr::from_str(&server.addr)
                .context("Failed to parse server address")?
                .ip(),
            port: server.gameport,
            valve_location: ValveServerLocation::parse(&server.name),
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
    }
}

impl Server {
    /// Fetch players for this server, if they haven't already been fetched. Mutates self to update the player data in place.
    pub(crate) async fn fetch_players(&mut self, client: &ApiClient) -> anyhow::Result<()> {
        if self.players.is_some() {
            return Ok(());
        }

        match self.ip {
            IpAddr::V4(ip) => {
                if ip.is_link_local() {
                    let players = client
                        .sdr_playerlist_query(SocketAddrV4::new(ip, self.port))
                        .await?;
                    self.players = Some(players);
                    Ok(())
                } else {
                    Err(anyhow!(
                        "We can only query players for servers behind Steam Datagram Relay (SDR)"
                    ))
                }
            }
            IpAddr::V6(_) => Err(anyhow!(
                "IPv6 not supported yet - we can only query players for servers behind Steam Datagram Relay (SDR)"
            )),
        }
    }
}

/// A server region. See https://developer.valvesoftware.com/wiki/Sv_region
/// Not the same as Valve's matchmaking regions for official servers
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
pub(crate) struct Player {
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

// These examples should all match (tests ensure this):
// "Valve Matchmaking Server (Sydney srcds1012-syd1 #7)" -> "syd1"
// "Valve Matchmaking Server (srcds2020-dfw2 #225)" -> "dfw2"
// "Valve Matchmaking Server (srcds1009-fsn-hetz #72)" -> "fsn-hetz"
static VALVE_SERVER_LOCATION_REGEX: LazyLock<Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^Valve Matchmaking Server \((?:[[:alpha:] ]+ )?srcds[[:digit:]]+-([[:lower:]]{3}[[:digit:][:lower:]-]+) #[[:digit:]]+\)$").expect("hardcoded regex should be valid")
});

/// Information about an official TF2 server (Valve-hosted) determined by parsing it's name.
/// Previously a struct with various fields, now just a semantically-meaningful String of the server's airport code.
#[derive(Debug, PartialEq, Eq, Clone, Serialize)]
pub(crate) struct ValveServerLocation(pub(crate) String);

impl ValveServerLocation {
    pub(crate) fn parse(s: impl AsRef<str>) -> Option<Self> {
        VALVE_SERVER_LOCATION_REGEX
            .captures(s.as_ref())
            .map(|captures| {
                ValveServerLocation(
                    captures
                        .get(1)
                        .expect("regex has at least one capture")
                        .as_str()
                        .into(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    #[test]
    #[allow(clippy::unreadable_literal)]
    fn test_fakeip_to_int() {
        assert_eq!(
            fakeip_to_int(Ipv4Addr::LOCALHOST),
            0b01111111_00000000_00000000_00000001
        );
    }

    #[test]
    fn test_server_location_from_str() {
        assert_eq!(
            ValveServerLocation::parse("Valve Matchmaking Server (Sydney srcds1013-syd1 #327)")
                .unwrap(),
            ValveServerLocation("syd1".into())
        );
        assert_eq!(
            ValveServerLocation::parse("Valve Matchmaking Server (srcds2020-dfw2 #225)").unwrap(),
            ValveServerLocation("dfw2".into())
        );
        assert_eq!(
            ValveServerLocation::parse("Valve Matchmaking Server (srcds1009-fsn-hetz #72)")
                .unwrap(),
            ValveServerLocation("fsn-hetz".into())
        );
    }
}
