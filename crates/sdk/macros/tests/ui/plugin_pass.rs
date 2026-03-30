//! Tests for the Plugin derive macro - successful cases.

use nebula_macros::Plugin;
include!("support.rs");

/// A simple unit struct plugin.
#[derive(Plugin)]
#[plugin(
    key = "http",
    name = "HTTP",
    description = "HTTP request actions"
)]
pub struct HttpPlugin;

/// A plugin with version and groups.
#[derive(Plugin)]
#[plugin(
    key = "database",
    name = "Database",
    description = "Database connection and query actions",
    version = 2,
    group = ["data", "storage"]
)]
pub struct DatabasePlugin;

fn main() {
    let http = HttpPlugin;
    let _ = http.metadata();

    let db = DatabasePlugin;
    let _ = db.metadata();
}
