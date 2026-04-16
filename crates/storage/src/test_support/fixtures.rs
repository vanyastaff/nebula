//! Factory functions for creating test data.
//!
//! These produce valid row structs with sensible defaults,
//! making it easy to write concise tests without specifying every field.

use chrono::Utc;

use crate::rows::*;

/// Generate a pseudo-unique 16-byte ID for tests.
///
/// Uses nanosecond timestamp mixed with an atomic counter,
/// producing IDs that are unique across calls within a process.
pub fn random_id() -> Vec<u8> {
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);

    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&nanos.to_le_bytes()[..8]);
    bytes[8..16].copy_from_slice(&seq.to_le_bytes());
    bytes.to_vec()
}

/// Create a test [`UserRow`] with the given email and generated defaults.
pub fn test_user(email: &str) -> UserRow {
    UserRow {
        id: random_id(),
        email: email.to_lowercase(),
        email_verified_at: None,
        display_name: email.split('@').next().unwrap_or("test-user").to_string(),
        avatar_url: None,
        password_hash: None,
        created_at: Utc::now(),
        last_login_at: None,
        locked_until: None,
        failed_login_count: 0,
        mfa_enabled: false,
        mfa_secret: None,
        version: 0,
        deleted_at: None,
    }
}

/// Create a test [`OrgRow`] with the given slug.
pub fn test_org(slug: &str) -> OrgRow {
    OrgRow {
        id: random_id(),
        slug: slug.to_string(),
        display_name: slug.to_string(),
        created_at: Utc::now(),
        created_by: random_id(),
        plan: "self_host".to_string(),
        billing_email: None,
        settings: serde_json::json!({}),
        version: 0,
        deleted_at: None,
    }
}

/// Create a test [`WorkspaceRow`] belonging to the given org.
pub fn test_workspace(org_id: &[u8], slug: &str) -> WorkspaceRow {
    WorkspaceRow {
        id: random_id(),
        org_id: org_id.to_vec(),
        slug: slug.to_string(),
        display_name: slug.to_string(),
        description: None,
        created_at: Utc::now(),
        created_by: random_id(),
        is_default: false,
        settings: serde_json::json!({}),
        version: 0,
        deleted_at: None,
    }
}

/// Create a test [`WorkflowRow`] in the given workspace.
pub fn test_workflow(workspace_id: &[u8], slug: &str) -> WorkflowRow {
    let now = Utc::now();
    WorkflowRow {
        id: random_id(),
        workspace_id: workspace_id.to_vec(),
        slug: slug.to_string(),
        display_name: slug.to_string(),
        description: None,
        current_version_id: random_id(),
        state: "Active".to_string(),
        created_at: now,
        created_by: random_id(),
        updated_at: now,
        version: 0,
        deleted_at: None,
    }
}

/// Create a test [`ExecutionRow`] for the given workspace, org, and version.
pub fn test_execution(workspace_id: &[u8], org_id: &[u8], version_id: &[u8]) -> ExecutionRow {
    ExecutionRow {
        id: random_id(),
        workspace_id: workspace_id.to_vec(),
        org_id: org_id.to_vec(),
        workflow_version_id: version_id.to_vec(),
        status: "Pending".to_string(),
        source: serde_json::json!({"kind": "Manual"}),
        input: None,
        output: None,
        vars: None,
        progress_summary: None,
        created_at: Utc::now(),
        scheduled_at: None,
        started_at: None,
        finished_at: None,
        claimed_by: None,
        claimed_until: None,
        cancel_requested_at: None,
        cancel_requested_by: None,
        cancel_reason: None,
        escalated: false,
        restarted_from: None,
        execution_timeout_at: None,
        version: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_id_is_16_bytes() {
        assert_eq!(random_id().len(), 16);
    }

    #[test]
    fn random_ids_are_unique() {
        let a = random_id();
        let b = random_id();
        assert_ne!(a, b);
    }

    #[test]
    fn test_user_defaults() {
        let user = test_user("alice@example.com");
        assert_eq!(user.email, "alice@example.com");
        assert_eq!(user.display_name, "alice");
        assert_eq!(user.version, 0);
        assert_eq!(user.id.len(), 16);
        assert!(!user.mfa_enabled);
        assert!(user.deleted_at.is_none());
    }

    #[test]
    fn test_org_defaults() {
        let org = test_org("acme");
        assert_eq!(org.slug, "acme");
        assert_eq!(org.plan, "self_host");
        assert!(org.deleted_at.is_none());
    }

    #[test]
    fn test_workspace_links_org() {
        let org = test_org("acme");
        let ws = test_workspace(&org.id, "default");
        assert_eq!(ws.org_id, org.id);
        assert_eq!(ws.slug, "default");
        assert!(!ws.is_default);
    }

    #[test]
    fn test_workflow_links_workspace() {
        let org = test_org("acme");
        let ws = test_workspace(&org.id, "default");
        let wf = test_workflow(&ws.id, "my-flow");
        assert_eq!(wf.workspace_id, ws.id);
        assert_eq!(wf.state, "Active");
        assert!(wf.deleted_at.is_none());
    }

    #[test]
    fn test_execution_links() {
        let org = test_org("acme");
        let ws = test_workspace(&org.id, "default");
        let version_id = random_id();
        let exec = test_execution(&ws.id, &org.id, &version_id);
        assert_eq!(exec.workspace_id, ws.id);
        assert_eq!(exec.org_id, org.id);
        assert_eq!(exec.workflow_version_id, version_id);
        assert_eq!(exec.status, "Pending");
        assert!(!exec.escalated);
    }
}
