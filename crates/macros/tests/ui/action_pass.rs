//! Tests for the Action derive macro - successful cases.

use nebula_macros::Action;
include!("support.rs");

/// A simple unit struct action.
#[derive(Action)]
#[action(
    key = "test.unit",
    name = "Unit Action",
    description = "A simple unit action for testing"
)]
pub struct UnitAction;

/// An action with configuration.
#[derive(Action)]
#[action(
    key = "http.request",
    name = "HTTP Request",
    description = "Make HTTP requests to external APIs",
    version = "2.1",
    action_type = "process",
    isolation = "sandbox"
)]
pub struct HttpRequestAction {
    #[action(config)]
    config: HttpConfig,
}

/// Action with credential requirement.
#[derive(Action)]
#[action(
    key = "slack.send",
    name = "Send Slack Message",
    description = "Send a message to a Slack channel",
    credential = "slack_oauth"
)]
pub struct SlackSendAction {
    #[action(config)]
    config: SlackConfig,
}

// Supporting types
#[derive(Debug, Default)]
pub struct HttpConfig {
    timeout: u64,
}

#[derive(Debug, Default)]
pub struct SlackConfig {
    channel: String,
}

fn main() {
    let unit = UnitAction;
    let _ = unit.metadata();

    let http = HttpRequestAction {
        config: HttpConfig { timeout: 30 },
    };
    let _ = http.metadata();

    let slack = SlackSendAction {
        config: SlackConfig {
            channel: "#general".to_string(),
        },
    };
    let _ = slack.metadata();
}
