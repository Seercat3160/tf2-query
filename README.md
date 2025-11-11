
# TF2Query

A tool for querying data about currently online Team Fortress 2 servers from the Steam API, including fetching their online players (currently only supported for servers behind SDR, so all Valve servers and some community servers).

## Usage

Running the program requires a valid Steam API key in the `STEAM_API_KEY` environment variable.
Get one from <https://steamcommunity.com/dev/apikey>.

I have tried to ensure rate limits are respected, but you are responsible for ensuring that you comply with the Steam API terms of service.

### Examples

#### Get all Valve servers in Sydney

```sh
tf2-query --valve --valve-location syd
```

#### Get all Valve servers worldwide, fetching their online players

Note: This may take a while to run, as it needs to query each server individually for online players. If not using `--get-players`, only one request is ever made (for the main server list).

```sh
tf2-query --valve --get-players
```

#### Query a server behind SDR for its online players by providing a "Fake IP"

For example, a server with address `169.254.125.252` (an SDR Fake IP) and port `31312`:

```sh
tf2-query sdr-query 169.254.125.252:31312
```

#### Query an Internet-accessible server for its online players and other info

Be aware that this directly connects to the server, so it will be able to see your IP address.

For example, a server with address `127.0.0.1` and port `27015`:

```sh
tf2-query a2s-query 127.0.0.1:27015
```

#### Get a summary of populated Valve casual servers in the Sydney region

```sh
tf2-query --has-players --no-mvm --valve --valve-location syd | jq --raw-output 'sort_by(.num_players) | reverse | [.[] | "\(.map) - \(.num_players)/\(.max_players) players"] | sort | .[]'
```

## Valve Servers

It is possible to filter for Valve servers from a specific region based on the "airport code" of the datacentre.
Specify this using `--valve-location code`, which matches any starting with `code`.

Here is a list of known valid locations, based on the in-game list under matchmaking settings. This list may not be exhaustive.

- Sydney, Australia: `syd`
- Singapore: `sgp`
- Hong Kong: `hkg`
- Ambattur, Chennai, India: `maa`
- Mumbai, India: `bom`
- Los Angeles, USA: `lax`
- Tokyo, Japan: `tyo`
- Seattle, USA: `sea`
- Dallas, USA: `dfw`
- Atlanta, USA: `atl`
- Chicago, USA: `ord`
- Sterling, Virginia, USA: `iad`
- Seoul, South Korea: `seo`
- Lima, Peru: `lim`
- Santiago, Chile: `scl`
- Dubai, UAE: `dxb`
- London, England: `lhr`
- Frankfurt, Germany: `fra`
- Falkenstein, Germany: `fsn`
- Madrid, Spain: `mad`
- Buenos Aires, Argentina: `eze`
- Vienna, Austria: `vie`
- Warsaw, Poland: `waw`
- Stockholm, Sweden: `sto`
- Sao Paulo, Brazil: `gru`
- Johannesburg, South Africa: `jnb`
- Helsinki, Finland: `hel`
