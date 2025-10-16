
# TF2Query

A tool for querying data about currently online Team Fortress 2 servers from the Steam API, including fetching their online players (currently only supported for servers behind SDR, so all Valve servers and some community servers).

## Usage

Running the program requires a valid Steam API key in the `STEAM_API_KEY` environment variable.
Get one from <https://steamcommunity.com/dev/apikey>.

I have tried to ensure rate limits are respected, but you are responsible for ensuring that you comply with the Steam API terms of service.

### Examples

#### Get all Valve servers in Sydney

```sh
tf2-query --valve --valve-region Sydney
```

#### Get all Valve servers worldwide, fetching their online players

Note: This may take a while to run, as it needs to query each server for it's online players. If not using `--get-players`, only one request is ever made, for the main server list.

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
tf2-query --has-players --no-mvm --valve --valve-region Sydney | jq --raw-output 'sort_by(.num_players) | reverse | [.[] | "\(.map) - \(.num_players)/\(.max_players) players"] | sort | .[]'
```
