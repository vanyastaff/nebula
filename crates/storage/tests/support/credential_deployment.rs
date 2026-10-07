//! A credential store and the tenant provisioning store on one deployment
//! database.
//!
//! The credential baseline requires a live workspace in the deployment database.
//! A composition hands the credential store the pool its
//! other stores use, so a test provisions tenants on a pool onto that same
//! database rather than through the credential store.

use nebula_storage::credential::SqliteCredentialPersistence;
use nebula_storage::sqlite::SqliteTenantProvisioningStore;

/// A fresh in-memory deployment database shared by a credential store and
/// a tenant provisioning store.
pub(crate) async fn memory_deployment()
-> (SqliteCredentialPersistence, SqliteTenantProvisioningStore) {
    let pool = nebula_storage::sqlite::open_memory_deployment()
        .await
        .expect("open an in-memory deployment database");
    let store = SqliteCredentialPersistence::connect_pool(pool.clone())
        .await
        .expect("an admitted SQLite credential store on the deployment pool");
    (store, SqliteTenantProvisioningStore::new(pool))
}

/// The tenant provisioning store on a pool of its own onto the SQLite
/// database at `url`, which a credential store has already admitted.
pub(crate) async fn file_tenants(url: &str) -> SqliteTenantProvisioningStore {
    let options = url
        .parse::<sqlx::sqlite::SqliteConnectOptions>()
        .expect("SQLite database locator")
        .create_if_missing(false)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("a tenancy pool on the deployment database");
    SqliteTenantProvisioningStore::new(pool)
}
