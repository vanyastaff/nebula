//! Users: global (no tenant scope); email is unique among active rows
//! (case-insensitive).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::UserRow;
use nebula_storage_port::store::UserStore;
use parking_lot::Mutex;

use super::{duplicate, version_conflict};

/// In-memory `users` store.
#[derive(Debug, Default, Clone)]
pub struct InMemoryUserStore {
    inner: Arc<Mutex<UserState>>,
}

#[derive(Debug, Default)]
struct UserState {
    rows: HashMap<String, Arc<UserRow>>,
    deleted_ids: HashSet<String>,
}

impl UserState {
    fn live(&self, id: &str) -> Option<&Arc<UserRow>> {
        self.rows
            .get(id)
            .filter(|user| !self.deleted_ids.contains(id) && user.deleted_at.is_none())
    }

    fn live_email_taken(&self, email: &str, except_id: Option<&str>) -> bool {
        self.rows.iter().any(|(id, user)| {
            Some(id.as_str()) != except_id
                && !self.deleted_ids.contains(id)
                && user.deleted_at.is_none()
                && user.email.eq_ignore_ascii_case(email)
        })
    }
}

impl InMemoryUserStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl UserStore for InMemoryUserStore {
    async fn create(&self, row: UserRow) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        if state.rows.contains_key(&row.id) {
            return Err(duplicate("user", "id"));
        }
        if state.live_email_taken(&row.email, None) {
            return Err(duplicate("user", "email"));
        }
        state.rows.insert(row.id.clone(), Arc::new(row));
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<Arc<UserRow>>, StorageError> {
        Ok(self.inner.lock().live(id).cloned())
    }

    async fn get_by_email(&self, email: &str) -> Result<Option<Arc<UserRow>>, StorageError> {
        let state = self.inner.lock();
        Ok(state
            .rows
            .iter()
            .find(|(id, user)| {
                !state.deleted_ids.contains(*id)
                    && user.deleted_at.is_none()
                    && user.email.eq_ignore_ascii_case(email)
            })
            .map(|(_, user)| Arc::clone(user)))
    }

    async fn update(&self, row: UserRow, expected_version: u64) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        let Some(current) = state.live(&row.id) else {
            return Err(StorageError::not_found("user", row.id));
        };
        if current.version != expected_version {
            let actual = current.version;
            return Err(version_conflict("user", row.id, expected_version, actual));
        }
        // An email change must not collide with another active user — the
        // create-path invariant, re-enforced on update.
        if state.live_email_taken(&row.email, Some(&row.id)) {
            return Err(duplicate("user", "email"));
        }
        state.rows.insert(row.id.clone(), Arc::new(row));
        Ok(())
    }

    async fn soft_delete(&self, id: &str) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        if state.live(id).is_none() {
            return Err(StorageError::not_found("user", id));
        }
        state.deleted_ids.insert(id.to_owned());
        Ok(())
    }
}
