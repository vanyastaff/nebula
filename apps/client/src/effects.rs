//! Network work runs outside rendering. Every request carries its session stamp, so a late
//! reply from a previous workspace or sign-in is rejected before it can change state.

use crate::{
    session::{RequestStamp, SessionContext},
    transport::{Connection, Failure, SignIn, SignedIn},
};
use eframe::egui;
use nebula_api_contract::v1::{
    catalog::{ActionParametersResponse, ListActionsResponse},
    execution::{ExecutionDetailResponse, ExecutionResponse, ListExecutionsResponse},
    workflow::{
        CreateWorkflowRequest, ListWorkflowsResponse, UpdateWorkflowDocumentRequest,
        WorkflowDocumentResponse,
    },
};
use std::sync::mpsc;

pub(crate) enum Operation {
    Connect(SignIn),
    List(usize),
    Create(CreateWorkflowRequest),
    Load(String),
    Save(String, UpdateWorkflowDocumentRequest),
    Publish(String, u64),
    Run(String, String),
    /// Recent runs of a workflow, and with an execution id that run's status too, read together
    /// because only one request is in flight at a time.
    History(String, Option<String>),
    Status(String),
    Actions,
    /// One action's parameter schema.
    Action(String),
}

/// What a completed request was for. Reducers use it to decide what a failure means for a draft.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestKind {
    Read,
    Create,
    Save,
    Publish,
    Run,
    Connect,
    Catalog,
    Schema,
    /// Recent runs, whose failure the runs panel shows in place of the list.
    History,
}

impl Operation {
    pub(crate) fn kind(&self) -> RequestKind {
        match self {
            Self::Connect(_) => RequestKind::Connect,
            Self::Create(_) => RequestKind::Create,
            Self::Save(..) => RequestKind::Save,
            Self::Publish(..) => RequestKind::Publish,
            Self::Run(..) => RequestKind::Run,
            Self::Actions => RequestKind::Catalog,
            Self::Action(_) => RequestKind::Schema,
            Self::History(..) => RequestKind::History,
            Self::List(_) | Self::Load(_) | Self::Status(_) => RequestKind::Read,
        }
    }
}

pub(crate) enum Reply {
    Connected(SignedIn),
    Listed(ListWorkflowsResponse),
    Created(WorkflowDocumentResponse),
    Loaded(WorkflowDocumentResponse),
    Saved(WorkflowDocumentResponse),
    Published(WorkflowDocumentResponse),
    Started(ExecutionResponse),
    /// The status read is separate from the list's: a run that cannot be read keeps the list.
    History(
        ListExecutionsResponse,
        Option<Result<Box<ExecutionDetailResponse>, Failure>>,
    ),
    Status(Box<ExecutionDetailResponse>),
    Actions(ListActionsResponse),
    Action(String, Box<ActionParametersResponse>),
}

pub(crate) type Completion = (RequestStamp, RequestKind, Result<Reply, Failure>);

async fn perform(
    connection: Connection,
    context: Option<SessionContext>,
    operation: Operation,
) -> Result<Reply, Failure> {
    // Signing in is the one request made before a workspace is chosen.
    if let Operation::Connect(intent) = operation {
        return connection.sign_in(intent).await.map(Reply::Connected);
    }
    let context = context.ok_or(Failure::Configuration)?;
    let org = context.organization.as_str();
    let workspace = context.workspace_selector.as_str();
    match operation {
        // Handled above; listed only because the match must name every operation.
        Operation::Connect(_) => Err(Failure::Configuration),
        Operation::List(page) => connection
            .list(org, workspace, page)
            .await
            .map(Reply::Listed),
        Operation::Create(request) => connection
            .create(org, workspace, &request)
            .await
            .map(Reply::Created),
        Operation::Load(id) => connection
            .load(org, workspace, &id)
            .await
            .map(Reply::Loaded),
        Operation::Save(id, request) => connection
            .save(org, workspace, &id, &request)
            .await
            .map(Reply::Saved),
        Operation::Publish(id, revision) => connection
            .publish(org, workspace, &id, revision)
            .await
            .map(Reply::Published),
        Operation::Run(id, key) => connection
            .run(org, workspace, &id, &key)
            .await
            .map(Reply::Started),
        Operation::History(id, execution) => {
            let history = connection.history(org, workspace, &id).await?;
            let status = match execution {
                Some(execution) => Some(
                    connection
                        .status(org, workspace, &execution)
                        .await
                        .map(Box::new),
                ),
                None => None,
            };
            Ok(Reply::History(history, status))
        },
        Operation::Status(id) => connection
            .status(org, workspace, &id)
            .await
            .map(|detail| Reply::Status(Box::new(detail))),
        Operation::Actions => connection.actions().await.map(Reply::Actions),
        Operation::Action(key) => connection
            .action_parameters(&key)
            .await
            .map(|schema| Reply::Action(key, Box::new(schema))),
    }
}

/// Owns the async runtime and the channel that carries completions back to the render loop.
pub(crate) struct Effects {
    sender: mpsc::Sender<Completion>,
    receiver: mpsc::Receiver<Completion>,
    #[cfg(not(target_arch = "wasm32"))]
    runtime: tokio::runtime::Runtime,
}

impl Effects {
    pub(crate) fn new() -> Result<Self, std::io::Error> {
        let (sender, receiver) = mpsc::channel();
        Ok(Self {
            sender,
            receiver,
            #[cfg(not(target_arch = "wasm32"))]
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?,
        })
    }

    /// Starts one request. The reducer decides later whether its reply still matters.
    pub(crate) fn start(
        &self,
        context: &egui::Context,
        stamp: RequestStamp,
        connection: Connection,
        session: Option<SessionContext>,
        operation: Operation,
    ) {
        let kind = operation.kind();
        let sender = self.sender.clone();
        let context = context.clone();
        let work = async move {
            let result = perform(connection, session, operation).await;
            let _ = sender.send((stamp, kind, result));
            context.request_repaint();
        };
        #[cfg(not(target_arch = "wasm32"))]
        self.runtime.spawn(work);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(work);
    }

    /// Completions that arrived since the last frame. Never blocks.
    pub(crate) fn completions(&self) -> impl Iterator<Item = Completion> + '_ {
        self.receiver.try_iter()
    }
}
