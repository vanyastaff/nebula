//! Tests for the Resource derive macro - successful cases.

use nebula_macros::Resource;
include!("support.rs");

/// A PostgreSQL database resource.
#[derive(Resource)]
#[resource(
    id = "postgres",
    config = PgConfig,
    instance = PgPool
)]
pub struct PostgresResource;

/// A Redis cache resource.
#[derive(Resource)]
#[resource(
    id = "redis",
    config = RedisConfig,
    instance = RedisConnection
)]
pub struct RedisResource;

// Supporting types
#[derive(Debug, Default)]
pub struct PgConfig {
    url: String,
}

#[derive(Debug, Default)]
pub struct PgPool;

#[derive(Debug, Default)]
pub struct RedisConfig {
    host: String,
    port: u16,
}

#[derive(Debug, Default)]
pub struct RedisConnection;

fn main() {
    let pg = PostgresResource;
    let _ = pg.id();

    let redis = RedisResource;
    let _ = redis.id();
}
