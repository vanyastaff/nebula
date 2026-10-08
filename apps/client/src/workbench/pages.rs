//! Where the user is in the workspace and what each page has read. A page's data is a [`Remote`]:
//! never asked for, on its way, shown, or failed with a reason. Pages draw each state the same way,
//! so loading, empty and error states look alike everywhere.

use nebula_api_contract::v1::{
    catalog::ActionDetailResponse,
    credential::{CredentialSummary, CredentialTypeInfo, TestCredentialResponse},
    execution::{ExecutionDetailResponse, ExecutionStatus, ExecutionSummary},
    me::{MeResponse, TokenSummary},
    org::MemberSummary,
    workflow::{WorkflowDocumentResponse, WorkflowResponse},
    workspace_membership::WorkspaceMemberSummary,
};
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap};
use zeroize::Zeroizing;

/// The pages of an open workspace. The editor is reached by opening a workflow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Page {
    #[default]
    Workflows,
    Editor,
    Executions,
    Catalog,
    Triggers,
    Credentials,
    Team,
    Settings,
}

impl Page {
    /// The pages the navigation lists, in its order; Alt with the position opens each.
    pub(crate) const NAVIGATION: [Self; 7] = [
        Self::Workflows,
        Self::Executions,
        Self::Catalog,
        Self::Triggers,
        Self::Credentials,
        Self::Team,
        Self::Settings,
    ];

    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Workflows | Self::Editor => "Workflows",
            Self::Executions => "Executions",
            Self::Catalog => "Node catalog",
            Self::Triggers => "Triggers",
            Self::Credentials => "Credentials",
            Self::Team => "Team",
            Self::Settings => "Settings",
        }
    }

    /// The navigation entry this page belongs to; the editor belongs to the workflows.
    pub(crate) const fn section(self) -> Self {
        match self {
            Self::Editor => Self::Workflows,
            other => other,
        }
    }
}

/// Data a page reads from the server.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Remote<T> {
    /// Not asked for yet; the page asks when it is shown.
    #[default]
    Idle,
    Loading,
    /// Being read again; the previous answer stays on screen meanwhile.
    Reloading(T),
    Ready(T),
    /// Out of date after a change; still shown, and read again on the next frame the page is shown.
    Stale(T),
    Failed(String),
}

impl<T> Remote<T> {
    pub(crate) const fn value(&self) -> Option<&T> {
        match self {
            Self::Ready(value) | Self::Reloading(value) | Self::Stale(value) => Some(value),
            Self::Idle | Self::Loading | Self::Failed(_) => None,
        }
    }

    pub(crate) const fn value_mut(&mut self) -> Option<&mut T> {
        match self {
            Self::Ready(value) | Self::Reloading(value) | Self::Stale(value) => Some(value),
            Self::Idle | Self::Loading | Self::Failed(_) => None,
        }
    }

    /// The data was read and nothing has changed it since: what is shown is the server's state.
    pub(crate) const fn settled(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    /// The page should ask for the data: it never was, or what is shown is out of date.
    pub(crate) const fn wants_read(&self) -> bool {
        matches!(self, Self::Idle | Self::Stale(_))
    }

    /// Marks a read as started, keeping what is shown.
    pub(crate) fn begin(&mut self) {
        *self = match std::mem::take(self) {
            Self::Ready(value) | Self::Reloading(value) | Self::Stale(value) => {
                Self::Reloading(value)
            },
            Self::Idle | Self::Loading | Self::Failed(_) => Self::Loading,
        };
    }

    /// Asks for the data again on the next frame the page is shown, keeping what is shown until
    /// the new answer arrives. A read already on its way is left to finish.
    pub(crate) fn invalidate(&mut self) {
        *self = match std::mem::take(self) {
            Self::Ready(value) | Self::Stale(value) => Self::Stale(value),
            loading @ (Self::Loading | Self::Reloading(_)) => loading,
            Self::Idle | Self::Failed(_) => Self::Idle,
        };
    }
}

/// Statuses the executions page can filter by, in the order its chips read.
pub(crate) const STATUS_FILTERS: [ExecutionStatus; 6] = [
    ExecutionStatus::Running,
    ExecutionStatus::Completed,
    ExecutionStatus::Failed,
    ExecutionStatus::Cancelled,
    ExecutionStatus::TimedOut,
    ExecutionStatus::Paused,
];

#[derive(Default)]
pub(crate) struct ExecutionsPage {
    pub(crate) list: Remote<Vec<ExecutionSummary>>,
    /// Cursor of the page after the shown ones, when there is one.
    pub(crate) next_cursor: Option<String>,
    /// The list is being extended with the next page rather than replaced.
    pub(crate) appending: bool,
    /// Why reading the next page failed; the rows read so far stay.
    pub(crate) more_error: Option<String>,
    pub(crate) statuses: BTreeSet<StatusFilter>,
    pub(crate) workflow: Option<String>,
    /// The filters of the read on its way, so an answer for filters changed since is read again.
    pub(crate) read_for: Option<(BTreeSet<StatusFilter>, Option<String>)>,
    /// Every workflow of the workspace, for the workflow filter; the Workflows page holds only one
    /// page of them.
    pub(crate) workflows: Remote<Vec<WorkflowResponse>>,
    pub(crate) selected: Option<String>,
    pub(crate) detail: Remote<Box<ExecutionDetailResponse>>,
    /// Node whose attempts and output the detail shows.
    pub(crate) node: Option<String>,
}

/// An execution status as a filter value; ordered so the chips and the query read the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct StatusFilter(pub(crate) u8);

impl StatusFilter {
    pub(crate) fn of(status: ExecutionStatus) -> Self {
        Self(
            STATUS_FILTERS
                .iter()
                .position(|known| *known == status)
                .unwrap_or(STATUS_FILTERS.len()) as u8,
        )
    }

    pub(crate) fn status(self) -> Option<ExecutionStatus> {
        STATUS_FILTERS.get(usize::from(self.0)).copied()
    }
}

#[derive(Default)]
pub(crate) struct CatalogPage {
    pub(crate) filter: String,
    pub(crate) selected: Option<String>,
    /// Each action's description, as it was read.
    pub(crate) details: HashMap<String, Remote<Box<ActionDetailResponse>>>,
    /// The action whose description is being read, so a failure knows which entry it settles.
    pub(crate) detail_request: Option<String>,
    /// Values typed into an action's form preview. They try the form out and go nowhere.
    pub(crate) preview: HashMap<String, Map<String, Value>>,
}

#[derive(Default)]
pub(crate) struct CredentialsPage {
    pub(crate) list: Remote<Vec<CredentialSummary>>,
    pub(crate) types: Remote<Vec<CredentialTypeInfo>>,
    /// The credential being created, until it is saved or abandoned.
    pub(crate) draft: Option<CredentialDraft>,
    /// The last test of each credential, by id.
    pub(crate) tests: HashMap<String, TestCredentialResponse>,
    /// A credential whose deletion waits for confirmation.
    pub(crate) confirm_delete: Option<String>,
}

/// A new credential: its type, name and the values typed into the type's form. The values hold
/// secrets, so they are wiped when the draft goes away.
pub(crate) struct CredentialDraft {
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) entries: Map<String, Value>,
}

impl Drop for CredentialDraft {
    fn drop(&mut self) {
        for value in self.entries.values_mut() {
            wipe(value);
        }
    }
}

/// Overwrites the strings of a value before it is freed.
fn wipe(value: &mut Value) {
    match value {
        Value::String(text) => zeroize::Zeroize::zeroize(text),
        Value::Array(items) => items.iter_mut().for_each(wipe),
        Value::Object(object) => object.values_mut().for_each(wipe),
        _ => {},
    }
}

#[derive(Default)]
pub(crate) struct TriggersPage {
    /// Every workflow's document, for the triggers bound in their definitions.
    pub(crate) documents: Remote<Vec<WorkflowDocumentResponse>>,
    /// A just-registered webhook: its address and signing secret, shown once.
    pub(crate) registered: Option<RegisteredWebhook>,
}

pub(crate) struct RegisteredWebhook {
    pub(crate) workflow: String,
    pub(crate) trigger: String,
    pub(crate) url: String,
    pub(crate) secret: Zeroizing<String>,
}

#[derive(Default)]
pub(crate) struct SettingsPage {
    pub(crate) profile: Remote<MeResponse>,
    /// The display name being edited.
    pub(crate) display_name: String,
    pub(crate) tokens: Remote<Vec<TokenSummary>>,
    pub(crate) new_token: NewToken,
    /// A just-created token, shown once.
    pub(crate) revealed: Option<(String, Zeroizing<String>)>,
    pub(crate) confirm_revoke: Option<String>,
}

pub(crate) struct NewToken {
    pub(crate) name: String,
    pub(crate) scopes: BTreeSet<String>,
    pub(crate) ttl_days: u32,
}

impl Default for NewToken {
    fn default() -> Self {
        Self {
            name: String::new(),
            scopes: BTreeSet::from(["workflows:read".to_owned()]),
            ttl_days: 90,
        }
    }
}

#[derive(Default)]
pub(crate) struct TeamPage {
    pub(crate) organization: Remote<Vec<MemberSummary>>,
    pub(crate) workspace: Remote<Vec<WorkspaceMemberSummary>>,
    pub(crate) new_member: String,
    pub(crate) new_role: String,
    pub(crate) new_workspace_role: String,
    pub(crate) confirm_remove: Option<String>,
}

/// Organization roles, most privileged first.
pub(crate) const ORG_ROLES: [&str; 4] = ["owner", "admin", "billing", "member"];
/// Workspace roles, most privileged first.
pub(crate) const WORKSPACE_ROLES: [&str; 4] = ["admin", "editor", "runner", "viewer"];
