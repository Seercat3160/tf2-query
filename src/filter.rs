use crate::api::Server;

use std::fmt::Write;

pub struct Filter {
    only_valve: bool,
    valve_location: Option<String>,
    name_filter_pattern: Option<String>,
    required_tags: Vec<String>,
    forbidden_tags: Vec<String>,
    min_players: i32,
}

impl Filter {
    // Info available at https://github.com/MegaAntiCheat/masterbase/blob/64ada88eff0d398ae229a44db2eeb8a31f00b126/masterbase/steam.py#L51 and https://developer.valvesoftware.com/wiki/Master_Server_Query_Protocol#Filter
    // Neither can be trusted to be correct, but it's a good starting point.
    // TODO: consider making a typed representation of the filter DSL (to use rather than doing string concat here to build the filter)
    pub(crate) fn build_api_filter_expression(&self) -> String {
        let mut filter_string = "\\appid\\440".to_string();

        if self.only_valve || self.valve_location.is_some() {
            let _ = write!(filter_string, "\\secure\\1\\linux\\1");
            // assumption: if multiple name_match operators are specified, they will all be used.
            let _ = write!(
                filter_string,
                "\\name_match\\Valve Matchmaking Server (*srcds*-{loc}* #*)",
                loc = self.valve_location.as_ref().unwrap_or(&String::new())
            );
        }

        if self.min_players > 0 {
            filter_string += "\\empty\\1"; // counter-intuitively, this excludes empty servers
        }

        if !self.forbidden_tags.is_empty() {
            // "nor" takes an argument that "Should specify the total size of the operand(s), meaning the number of \op\operand pairs" (according to the error message I got from the API in the x-error-message header).
            // masterbase code says gametype matches servers with *any* of the given tags, so negating that means none of them (this contradicts the wiki) // TODO: test this
            let _ = write!(
                filter_string,
                "\\nor\\1\\gametype\\{}",
                self.forbidden_tags.join(",")
            );
        }

        if !self.required_tags.is_empty() {
            // TODO: test whether gametype requires matching all of the given tags, or just any
            let _ = write!(
                filter_string,
                "\\gametype\\{}",
                self.required_tags.join("\\gametype\\")
            );
        }

        if let Some(pattern) = &self.name_filter_pattern {
            let _ = write!(filter_string, "\\name_match\\{pattern}");
        }

        filter_string
    }

    /// Determine whether or not a given server matches the filter.
    /// Assumes the server was returned from an API call where the filter expression built from this filter was used (i.e. some filtering may have already been done and need not be repeated).
    pub(crate) fn filter_server(&self, server: &Server) -> bool {
        if server.num_players < self.min_players {
            return false;
        }

        // TODO: is this simple approach too slow?
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

pub struct FilterBuilder {
    only_valve: bool,
    valve_location: Option<String>,
    name_filter_pattern: Option<String>,
    required_tags: Vec<String>,
    forbidden_tags: Vec<String>,
    min_players: i32,
}

impl FilterBuilder {
    #[must_use]
    pub fn build(self) -> Filter {
        Filter {
            only_valve: self.only_valve,
            valve_location: self.valve_location,
            name_filter_pattern: self.name_filter_pattern,
            required_tags: self.required_tags,
            forbidden_tags: self.forbidden_tags,
            min_players: self.min_players,
        }
    }

    #[must_use]
    pub fn only_valve(mut self, only_valve: bool) -> Self {
        self.only_valve = only_valve;
        self
    }

    #[must_use]
    pub fn valve_location(mut self, valve_location: Option<String>) -> Self {
        self.valve_location = valve_location;
        self
    }

    #[must_use]
    pub fn name_filter_pattern(mut self, name_filter_pattern: Option<String>) -> Self {
        self.name_filter_pattern = name_filter_pattern;
        self
    }

    #[must_use]
    pub fn required_tags(mut self, required_tags: Vec<String>) -> Self {
        self.required_tags = required_tags;
        self
    }

    #[must_use]
    pub fn forbidden_tags(mut self, forbidden_tags: Vec<String>) -> Self {
        self.forbidden_tags = forbidden_tags;
        self
    }

    #[must_use]
    pub fn min_players(mut self, min_players: i32) -> Self {
        self.min_players = min_players;
        self
    }
}

impl FilterBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            only_valve: false,
            valve_location: None,
            name_filter_pattern: None,
            required_tags: vec![],
            forbidden_tags: vec![],
            min_players: 0,
        }
    }
}

impl Default for FilterBuilder {
    fn default() -> Self {
        FilterBuilder::new()
    }
}
