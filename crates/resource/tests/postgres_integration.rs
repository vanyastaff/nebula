//! Integration tests for PostgreSQL resource with real database
//!
//! These tests use testcontainers to spin up a real PostgreSQL instance

#![cfg(all(test, feature = "postgres"))]

use nebula_resource::{
    core::{
        context::ResourceContext,
        resource::{Resource, ResourceId},
        traits::HealthCheckable,
    },
    resources::database::{DatabaseConfig, DatabaseResource},
};
use testcontainers::{clients::Cli, images::postgres::Postgres};

#[tokio::test]
async fn test_postgres_connection() {
    let docker = Cli::default();
    let postgres = docker.run(Postgres::default());
    let port = postgres.get_host_port_ipv4(5432);

    let config = DatabaseConfig {
        url: format!("postgres://postgres:postgres@localhost:{}/postgres", port),
        max_connections: 5,
        min_connections: 1,
        timeout_seconds: 10,
    };

    let resource = DatabaseResource;
    let context = ResourceContext::default();

    // Create instance
    let instance = resource.create(&config, &context).await.unwrap();

    // Verify instance is created
    assert_eq!(instance.resource_id(), &ResourceId::new("database", "1.0"));

    // Cleanup
    resource.cleanup(instance).await.unwrap();
}

#[tokio::test]
async fn test_postgres_query_execution() {
    let docker = Cli::default();
    let postgres = docker.run(Postgres::default());
    let port = postgres.get_host_port_ipv4(5432);

    let config = DatabaseConfig {
        url: format!("postgres://postgres:postgres@localhost:{}/postgres", port),
        max_connections: 5,
        min_connections: 1,
        timeout_seconds: 10,
    };

    let resource = DatabaseResource;
    let context = ResourceContext::default();

    let instance = resource.create(&config, &context).await.unwrap();

    // Execute CREATE TABLE
    let result = instance
        .execute_query("CREATE TABLE test_table (id SERIAL PRIMARY KEY, name TEXT)")
        .await
        .unwrap();
    assert_eq!(result, 0); // CREATE TABLE returns 0 rows affected

    // Execute INSERT
    let result = instance
        .execute_query("INSERT INTO test_table (name) VALUES ('test1'), ('test2')")
        .await
        .unwrap();
    assert_eq!(result, 2);

    // Execute SELECT (rows_affected returns 0 for SELECT)
    let result = instance
        .execute_query("SELECT * FROM test_table")
        .await
        .unwrap();
    assert_eq!(result, 0);

    resource.cleanup(instance).await.unwrap();
}

#[tokio::test]
async fn test_postgres_health_check() {
    let docker = Cli::default();
    let postgres = docker.run(Postgres::default());
    let port = postgres.get_host_port_ipv4(5432);

    let config = DatabaseConfig {
        url: format!("postgres://postgres:postgres@localhost:{}/postgres", port),
        max_connections: 5,
        min_connections: 1,
        timeout_seconds: 10,
    };

    let resource = DatabaseResource;
    let context = ResourceContext::default();

    let instance = resource.create(&config, &context).await.unwrap();

    // Basic health check
    let health = instance.health_check().await.unwrap();
    assert!(health.is_usable());
    assert_eq!(health.score(), 1.0);

    // Detailed health check
    let detailed = instance.detailed_health_check(&context).await.unwrap();
    assert!(detailed.is_usable());
    assert!(detailed.latency.is_some());
    assert!(detailed.metadata.contains_key("version"));
    assert!(detailed.metadata.contains_key("database"));
    assert!(detailed.metadata.contains_key("user"));

    resource.cleanup(instance).await.unwrap();
}

#[tokio::test]
async fn test_postgres_transactions() {
    let docker = Cli::default();
    let postgres = docker.run(Postgres::default());
    let port = postgres.get_host_port_ipv4(5432);

    let config = DatabaseConfig {
        url: format!("postgres://postgres:postgres@localhost:{}/postgres", port),
        max_connections: 5,
        min_connections: 1,
        timeout_seconds: 10,
    };

    let resource = DatabaseResource;
    let context = ResourceContext::default();

    let instance = resource.create(&config, &context).await.unwrap();

    // Create table
    instance
        .execute_query("CREATE TABLE txn_test (id SERIAL PRIMARY KEY, value INT)")
        .await
        .unwrap();

    // Begin transaction
    let mut txn = instance.begin_transaction().await.unwrap();

    // Execute within transaction
    let result = sqlx::query("INSERT INTO txn_test (value) VALUES (100)")
        .execute(&mut *txn)
        .await
        .unwrap();
    assert_eq!(result.rows_affected(), 1);

    // Commit transaction
    txn.commit().await.unwrap();

    // Verify data was committed
    let row: (i32,) = sqlx::query_as("SELECT COUNT(*) FROM txn_test")
        .fetch_one(instance.pool())
        .await
        .unwrap();
    assert_eq!(row.0, 1);

    resource.cleanup(instance).await.unwrap();
}

#[tokio::test]
async fn test_postgres_connection_pool() {
    let docker = Cli::default();
    let postgres = docker.run(Postgres::default());
    let port = postgres.get_host_port_ipv4(5432);

    let config = DatabaseConfig {
        url: format!("postgres://postgres:postgres@localhost:{}/postgres", port),
        max_connections: 3,
        min_connections: 1,
        timeout_seconds: 10,
    };

    let resource = DatabaseResource;
    let context = ResourceContext::default();

    let instance = resource.create(&config, &context).await.unwrap();

    // Verify pool configuration
    let pool = instance.pool();
    assert_eq!(pool.size(), 0); // No connections created yet

    // Execute query to create connection
    instance.execute_query("SELECT 1").await.unwrap();

    // Pool should have at least min_connections
    assert!(pool.size() > 0);

    resource.cleanup(instance).await.unwrap();
}
