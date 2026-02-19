use std::time::Duration;

use tracing::{error, info};

use crate::{
    api::{ApiClient, ApiError, ClientCreationError, Server},
    filter::Filter,
};

pub mod api;
pub mod filter;

/// Public interface used to query the Steam API
pub struct QueryClient {
    client: ApiClient,
}

impl QueryClient {
    /// Initialize an API client with a given Steam API key.
    ///
    /// # Errors
    ///
    /// This function will return an error if the key is invalid, or if validation or client creation fails in some other way.
    pub async fn new(steam_api_key: String) -> Result<Self, ClientCreationError> {
        let api = ApiClient::new(steam_api_key).await?;

        Ok(Self { client: api })
    }
}

impl QueryClient {
    /// Fetch a filtered serverlist from the Steam API.
    ///
    /// # Errors
    ///
    /// This function will return an error if the serverlist cannot be fetched.
    pub async fn serverlist(
        &self,
        server_filter: Filter,
        fetch_players: bool,
    ) -> Result<Vec<Server>, ApiError> {
        let serverlist_unfiltered = self
            .client
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
            // if should_loop_exit.is_cancelled() {
            //     break;
            // }

            if fetch_players && server.num_players.is_positive() {
                // select! {
                //     () = should_loop_exit.cancelled() => break,
                //     _ = interval.tick() => {}
                // }
                interval.tick().await;

                info!(
                    "({} of {}) - fetching players for {} ({}:{})...",
                    index + 1,
                    num_filtered_servers,
                    server.name,
                    server.ip,
                    server.port
                );

                match server.fetch_players(&self.client).await {
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

        Ok(filtered_servers)
    }
}
