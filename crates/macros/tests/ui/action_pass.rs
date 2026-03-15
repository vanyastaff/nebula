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

/// Action with version.
#[derive(Action)]
#[action(
    key = "http.request",
    name = "HTTP Request",
    description = "Make HTTP requests to external APIs",
    version = "2.1"
)]
pub struct HttpRequestAction;

/// Action with credential (string key is ignored; use credential = Type for components).
#[derive(Action)]
#[action(
    key = "slack.send",
    name = "Send Slack Message",
    description = "Send a message to a Slack channel",
    credential = "slack_oauth"
)]
pub struct SlackSendAction;

/// Action with credential and resource types (type-based components).
#[derive(Action)]
#[action(
    key = "db.query",
    name = "Database Query",
    description = "Execute a query against the database",
    credential = SlackOAuthCredential,
    resources = [PostgresDb, RedisCache]
)]
pub struct DbQueryAction;

// Credential and resource types for component tests
#[derive(Default)]
pub struct SlackOAuthCredential;
#[derive(Default)]
pub struct PostgresDb;
#[derive(Default)]
pub struct RedisCache;

fn main() {
    let unit = UnitAction;
    let _ = unit.metadata();

    let http = HttpRequestAction;
    let _ = http.metadata();

    let slack = SlackSendAction;
    let _ = slack.metadata();

    let db = DbQueryAction;
    let _ = db.metadata();
    // Verify ActionDependencies methods exist
    let _ = DbQueryAction::credential();
    let _ = DbQueryAction::resources();
}
