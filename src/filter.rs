use crate::{Args, api::Server};

use std::fmt::Write;

pub(crate) struct Filter {
    only_valve: bool,
    valve_location: Option<String>,
    name_filter_pattern: Option<String>,
    required_tags: Vec<String>,
    forbidden_tags: Vec<String>,
    min_players: i32,
}

impl Filter {
    pub(crate) fn new(args: &Args) -> Self {
        let mut required_tags = vec![];
        let mut forbidden_tags = vec![];
        let mut only_valve: bool = false;
        let mut min_players: i32 = 0;
        let mut name_filter: Option<String> = args.filter_name.clone();

        if args.no_mvm {
            forbidden_tags.push("mvm".into());
        }

        if args.valve || args.valve_location.is_some() {
            only_valve = true;
            required_tags.push("valve".into());

            let name_pattern = format!(
                "Valve Matchmaking Server (*srcds*-{}* #*)",
                args.valve_location.clone().unwrap_or_default(), // we postfix with a wildcard anyway, so we want to default to the empty string
            );
            name_filter = Some(name_pattern);
        }

        if args.has_players {
            min_players = 1;
        }

        Self {
            only_valve,
            valve_location: args.valve_location.clone(),
            name_filter_pattern: name_filter,
            required_tags,
            forbidden_tags,
            min_players,
        }
    }

    // Info available at https://github.com/MegaAntiCheat/masterbase/blob/64ada88eff0d398ae229a44db2eeb8a31f00b126/masterbase/steam.py#L51 and https://developer.valvesoftware.com/wiki/Master_Server_Query_Protocol#Filter
    // Neither can be trusted to be correct, but it's a good starting point.
    pub(crate) fn build_api_filter_expression(&self) -> String {
        let mut filter_string = "\\appid\\440".to_string();

        if self.only_valve {
            let _ = write!(filter_string, "\\secure\\1\\linux\\1");
        }
        if self.min_players > 0 {
            filter_string += "\\empty\\1"; // counter-intuitively, this excludes empty servers
        }

        if !self.forbidden_tags.is_empty() {
            // masterbase code says gametype matches servers with *any* of the given tags, so negating that means none of them (this contradicts the wiki) // TODO: test this
            let _ = write!(
                filter_string,
                "\\nor\\1\\gametype\\{}",
                self.forbidden_tags.join(",")
            );
        }

        // "nor" takes an argument that "Should specify the total size of the operand(s), meaning the number of \op\operand pairs" (according to the error message I got from the API in the x-error-message header)
        if !self.required_tags.is_empty() {
            // TODO: test whether gametype requires matching all of the given tags, or just any
            let _ = write!(
                filter_string,
                "\\gametype\\{}",
                self.required_tags.join("\\gametype\\")
            );
        }

        if let Some(pattern) = &self.name_filter_pattern {
            // name_match
            let _ = write!(filter_string, "\\name_match\\{pattern}");
        }

        filter_string
    }

    /// Determine whether or not a given server matches the filter. Assumes the server was returned from an API call whether the filter expression built from this filter was used (i.e. some filtering may have already been done and need not be repeated).
    pub(crate) fn filter_server(&self, server: &Server) -> bool {
        if server.num_players < self.min_players {
            return false;
        }

        // These tag filters should be improved, they might be slow
        for tag in &self.forbidden_tags {
            if server.tags.contains(tag) {
                return false;
            }
        }

        for tag in &self.required_tags {
            if !server.tags.contains(tag) {
                return false;
            }
        }

        if self.only_valve {
            // check that the name matches the regex
            let Some(ref location) = server.valve_location else {
                return false;
            };

            if let Some(filter_location) = &self.valve_location
                && !location.0.starts_with(filter_location)
            {
                return false;
            }
        }

        true
    }
}
