
# TF2Query

A tool for querying data about currently online Team Fortress 2 servers from the Steam API, including fetching their online players (currently only supported for servers behind SDR, so all Valve servers and some community servers).

## Usage

Running the program requires a valid Steam API key in the `STEAM_API_KEY` environment variable.
Get one from <https://steamcommunity.com/dev/apikey>.

I have tried my best to ensure rate limits are respected, but you are responsible for ensuring that you comply with the Steam API terms of service.

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
