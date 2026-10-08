//! What the demo workspace starts with: the signed-in user and their team, the seed workflows of
//! `workflows.json` stored the way the server stores a definition, their past runs played by the
//! simulated executor, and credentials and tokens of the kinds a production workspace holds.

use super::{StoredWorkflow, World, executor::Run};
use crate::clock;
use nebula_api_contract::v1::{
    credential::{CredentialLifecycleState, CredentialResponse},
    me::{MeResponse, TokenSummary},
    org::MemberSummary,
    shared::{OrgRoleDto, WorkspaceRoleDto},
    workflow::{WorkflowDocumentResponse, WorkflowResponse},
    workspace_membership::WorkspaceMemberSummary,
};
use serde_json::{Value, json};
use std::collections::HashMap;

/// The signed-in demo user.
pub(super) const ME: &str = "usr_01HZ8K2M4N6P8Q0R2S4T6V8W0X";
const GRACE: &str = "usr_01HZ8K3A7B9C1D3E5F7G9H1J3K";
const LINUS: &str = "usr_01HZ8K4B2C4D6E8F0G2H4J6K8M";
const MARGARET: &str = "usr_01HZ8K5C3D5E7F9G1H3J5K7M9N";
const KEN: &str = "usr_01HZ8K6D4E6F8G0H2J4K6M8N0P";

const DAY_MS: i64 = 86_400_000;

#[derive(serde::Deserialize)]
struct Seed {
    workflows: Vec<SeedWorkflow>,
}

#[derive(serde::Deserialize)]
struct SeedWorkflow {
    name: String,
    description: String,
    published: bool,
    sample_input: Value,
    definition: Value,
    history: Vec<PastRun>,
}

#[derive(serde::Deserialize)]
struct PastRun {
    minutes_ago: i64,
    /// A payload other than the sample, such as one that makes a node fail.
    #[serde(default)]
    input: Option<Value>,
    /// Cancels the run this long after it started.
    #[serde(default)]
    cancel_after_ms: Option<i64>,
}

pub(super) fn me() -> MeResponse {
    MeResponse {
        user_id: ME.to_owned(),
        email: "ada@acme.example".to_owned(),
        display_name: "Ada Lovelace".to_owned(),
        email_verified: true,
        mfa_enabled: true,
        orgs_count: Some(1),
        tokens_count: 0,
    }
}

pub(super) fn org_members() -> Vec<MemberSummary> {
    [
        (ME, "owner"),
        (GRACE, "admin"),
        (LINUS, "member"),
        (MARGARET, "billing"),
    ]
    .into_iter()
    .map(|(principal, role)| MemberSummary {
        principal_id: principal.to_owned(),
        role: OrgRoleDto(role.to_owned()),
    })
    .collect()
}

pub(super) fn workspace_members() -> Vec<WorkspaceMemberSummary> {
    [
        (ME, "admin"),
        (GRACE, "editor"),
        (LINUS, "runner"),
        (KEN, "viewer"),
    ]
    .into_iter()
    .map(|(principal, role)| WorkspaceMemberSummary {
        principal_id: principal.to_owned(),
        role: WorkspaceRoleDto(role.to_owned()),
    })
    .collect()
}

/// A workflow document as the API returns it.
pub(super) fn document(
    id: &str,
    name: &str,
    definition: Value,
    timestamp: i64,
    revision: u64,
) -> WorkflowDocumentResponse {
    WorkflowDocumentResponse {
        workflow: WorkflowResponse {
            id: id.to_owned(),
            name: name.to_owned(),
            description: definition["description"].as_str().map(str::to_owned),
            created_at: timestamp,
            updated_at: timestamp,
        },
        definition,
        revision,
    }
}

/// A definition in the shape the server stores: every field the engine defaults, filled in.
fn stored_definition(id: &str, seed: &SeedWorkflow, created: i64) -> Value {
    let nodes: Vec<Value> = seed.definition["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|node| {
            let mut stored = json!({
                "description": null, "enabled": true, "interface_version": null,
                "rate_limit": null, "retry_policy": null, "timeout": null
            });
            if let (Some(stored), Some(node)) = (stored.as_object_mut(), node.as_object()) {
                stored.extend(node.clone());
            }
            stored
        })
        .collect();
    let at = clock::rfc3339(created);
    json!({
        "id": id,
        "name": seed.name,
        "description": seed.description,
        "version": {"major": 0, "minor": 1, "patch": 0},
        "nodes": nodes,
        "connections": seed.definition["connections"],
        "trigger_bindings": seed.definition.get("trigger_bindings").cloned().unwrap_or_else(|| json!([])),
        "variables": {},
        "config": {
            "checkpointing": {"enabled": true, "interval": null},
            "error_strategy": "fail_fast",
            "max_parallel_nodes": 10,
            "retry_policy": null,
            "timeout": null
        },
        "tags": [],
        "owner_id": ME,
        "ui_metadata": seed.definition["ui_metadata"],
        "schema_version": 2,
        "created_at": at,
        "updated_at": at
    })
}

/// Fills the world with the seed workflows and their history, credentials and tokens.
pub(super) fn populate(world: &mut World, now: i64) {
    // The fixture is part of the build; the tests prove it parses. A broken one leaves the demo
    // empty rather than failing the sign-in.
    let seed: Seed = serde_json::from_str(include_str!("workflows.json")).unwrap_or(Seed {
        workflows: Vec::new(),
    });
    for (index, workflow) in seed.workflows.iter().enumerate() {
        let created = now - (index as i64 + 2) * 4 * DAY_MS;
        let id = world.next_id("wf", created);
        let definition = stored_definition(&id, workflow, created);
        // A published workflow was saved and then activated; a draft only created.
        let revision = if workflow.published { 3 } else { 1 };
        let mut document = document(
            &id,
            &workflow.name,
            definition.clone(),
            created / 1000,
            revision,
        );
        document.workflow.updated_at = (now - (index as i64 + 1) * DAY_MS / 3) / 1000;
        for past in &workflow.history {
            let at = now - past.minutes_ago * 60_000;
            let input = past
                .input
                .clone()
                .unwrap_or_else(|| workflow.sample_input.clone());
            let run_id = world.next_id("exe", at);
            let mut run = Run::plan(run_id, id.clone(), &definition, Some(input), at);
            if let Some(after) = past.cancel_after_ms {
                run.cancel(at + after);
            }
            world.runs.push(run);
        }
        world.workflows.push(StoredWorkflow {
            document,
            published: workflow.published.then_some(definition),
            sample_input: workflow.sample_input.clone(),
        });
    }
    populate_credentials(world, now);
    populate_tokens(world, now);
}

fn populate_credentials(world: &mut World, now: i64) {
    let seeded = [
        (
            "Stripe (live)",
            "api_key",
            41,
            CredentialLifecycleState::Ready,
        ),
        (
            "Warehouse SFTP",
            "basic_auth",
            63,
            CredentialLifecycleState::Ready,
        ),
        (
            "Google Sheets",
            "oauth2",
            12,
            CredentialLifecycleState::ReauthRequired,
        ),
        (
            "Billing webhooks",
            "signing_key",
            8,
            CredentialLifecycleState::Ready,
        ),
    ];
    for (name, kind, days_ago, lifecycle) in seeded {
        let Some(info) = world
            .credential_types
            .iter()
            .find(|info| info.key == kind)
            .cloned()
        else {
            continue;
        };
        let created = now - days_ago * DAY_MS;
        let id = world.next_id("cred", created);
        world.credentials.push(CredentialResponse {
            id,
            credential_key: info.key,
            name: name.to_owned(),
            description: None,
            auth_pattern: info.auth_pattern,
            capabilities: info.capabilities,
            created_at: clock::rfc3339(created),
            updated_at: clock::rfc3339(created + DAY_MS),
            expires_at: (kind == "oauth2").then(|| clock::rfc3339(now - DAY_MS)),
            version: 2,
            lifecycle,
            tags: HashMap::new(),
        });
    }
}

fn populate_tokens(world: &mut World, now: i64) {
    let seeded = [
        (
            "CI deploy",
            vec!["workflows:write", "executions:read"],
            30,
            Some(2),
        ),
        ("Local CLI", vec!["workflows:read"], 90, None),
    ];
    for (name, scopes, days_ago, used_hours_ago) in seeded {
        let created = now - days_ago * DAY_MS;
        let id = world.next_id("pat", created);
        world.tokens.push(TokenSummary {
            id,
            name: name.to_owned(),
            scopes: scopes.into_iter().map(str::to_owned).collect(),
            created_at: clock::rfc3339(created),
            last_used_at: used_hours_ago.map(|hours| clock::rfc3339(now - hours * 3_600_000)),
            expires_at: Some(clock::rfc3339(created + 365 * DAY_MS)),
        });
    }
}
