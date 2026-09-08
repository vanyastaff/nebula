//! Private PostgreSQL schemas for engine reconnect oracles.

pub(crate) async fn connect_with_private_schema(
    url: &str,
    prefix: &str,
) -> Result<sqlx::PgPool, sqlx::Error> {
    assert!(
        prefix
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    );
    let schema = format!("{prefix}_{}", ulid::Ulid::new().to_string().to_lowercase());
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await?;
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await?;
    admin.close().await;
    let options = url
        .parse::<sqlx::postgres::PgConnectOptions>()?
        .options([("search_path", schema)]);
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
}
