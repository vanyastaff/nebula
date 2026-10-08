//! Work outside rendering. Requests go through the typed API ([`Backend`]) one at a time, and each
//! carries its session stamp, so a late reply from a previous workspace or sign-in is rejected
//! before it can change state. Watching an execution is separate: a stream of its states that runs
//! beside the requests until the execution ends or the watch is replaced.

use crate::{
    api::{Backend, SignedIn},
    clock,
    session::{RequestStamp, SessionContext},
    transport::{Failure, SignIn},
};
use eframe::egui;
use nebula_api_contract::v1::{
    catalog::{ActionParametersResponse, ListActionsResponse},
    execution::{ExecutionDetailResponse, ExecutionResponse, ExecutionStatus, ListExecutionsResponse},
    workflow::{
        CreateWorkflowRequest, ListWorkflowsResponse, UpdateWorkflowDocumentRequest,
        WorkflowDocumentResponse,
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

pub(crate) enum Operation {
    /// Signs in to a server with these credentials, or opens the demo without any.
    Connect(Option<SignIn>),
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

/// One state of a watched execution, tagged with the session generation it was read in.
pub(crate) type WatchEvent = (u64, Result<Box<ExecutionDetailResponse>, Failure>);

async fn perform(
    backend: Backend,
    context: Option<SessionContext>,
    operation: Operation,
) -> Result<Reply, Failure> {
    // Signing in is the one request made before a workspace is chosen.
    if let Operation::Connect(intent) = operation {
        return backend.sign_in(intent).await.map(Reply::Connected);
    }
    let scope = context.ok_or(Failure::Configuration)?;
    match operation {
        // Handled above; listed only because the match must name every operation.
        Operation::Connect(_) => Err(Failure::Configuration),
        Operation::List(page) => backend.workflows(&scope, page).await.map(Reply::Listed),
        Operation::Create(request) => backend
            .create_workflow(&scope, &request)
            .await
            .map(Reply::Created),
        Operation::Load(id) => backend.workflow(&scope, &id).await.map(Reply::Loaded),
        Operation::Save(id, request) => backend
            .save_workflow(&scope, &id, &request)
            .await
            .map(Reply::Saved),
        Operation::Publish(id, revision) => backend
            .publish_workflow(&scope, &id, revision)
            .await
            .map(Reply::Published),
        Operation::Run(id, key) => backend.start(&scope, &id, &key).await.map(Reply::Started),
        Operation::History(id, execution) => {
            let history = backend.workflow_history(&scope, &id).await?;
            let status = match execution {
                Some(execution) => Some(backend.execution(&scope, &execution).await.map(Box::new)),
                None => None,
            };
            Ok(Reply::History(history, status))
        },
        Operation::Status(id) => backend
            .execution(&scope, &id)
            .await
            .map(|detail| Reply::Status(Box::new(detail))),
        Operation::Actions => backend.actions().await.map(Reply::Actions),
        Operation::Action(key) => backend
            .action_parameters(&key)
            .await
            .map(|schema| Reply::Action(key, Box::new(schema))),
    }
}

/// An execution being watched, and the flag that stops its stream.
struct Watch {
    execution: String,
    stop: Arc<AtomicBool>,
}

/// Owns the async runtime and the channels that carry completions and watched states back to the
/// render loop.
pub(crate) struct Effects {
    sender: mpsc::Sender<Completion>,
    receiver: mpsc::Receiver<Completion>,
    watch_sender: mpsc::Sender<WatchEvent>,
    watch_receiver: mpsc::Receiver<WatchEvent>,
    watch: Option<Watch>,
    #[cfg(not(target_arch = "wasm32"))]
    runtime: tokio::runtime::Runtime,
}

impl Effects {
    pub(crate) fn new() -> Result<Self, std::io::Error> {
        let (sender, receiver) = mpsc::channel();
        let (watch_sender, watch_receiver) = mpsc::channel();
        Ok(Self {
            sender,
            receiver,
            watch_sender,
            watch_receiver,
            watch: None,
            #[cfg(not(target_arch = "wasm32"))]
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?,
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn spawn(&self, work: impl std::future::Future<Output = ()> + Send + 'static) {
        self.runtime.spawn(work);
    }

    /// The browser runs futures on its one thread, so they need not be `Send`.
    #[cfg(target_arch = "wasm32")]
    fn spawn(&self, work: impl std::future::Future<Output = ()> + 'static) {
        wasm_bindgen_futures::spawn_local(work);
    }

    /// Starts one request. The reducer decides later whether its reply still matters.
    pub(crate) fn start(
        &self,
        context: &egui::Context,
        stamp: RequestStamp,
        backend: Backend,
        session: Option<SessionContext>,
        operation: Operation,
    ) {
        let kind = operation.kind();
        let sender = self.sender.clone();
        let context = context.clone();
        self.spawn(async move {
            let result = perform(backend, session, operation).await;
            // A closed channel means the app is shutting down; nobody is left to tell.
            let _delivered = sender.send((stamp, kind, result));
            context.request_repaint();
        });
    }

    /// Completions that arrived since the last frame. Never blocks.
    pub(crate) fn completions(&self) -> impl Iterator<Item = Completion> + '_ {
        self.receiver.try_iter()
    }

    /// The execution being watched, if any.
    pub(crate) fn watched(&self) -> Option<&str> {
        self.watch.as_ref().map(|watch| watch.execution.as_str())
    }

    /// Streams `execution`'s states until it ends, the watch is replaced, or a read fails. The
    /// demo's executor is read every frame or two; a server, which offers no push channel, is read
    /// once a second. Only changed states are delivered.
    pub(crate) fn watch(
        &mut self,
        context: &egui::Context,
        generation: u64,
        backend: Backend,
        session: SessionContext,
        execution: String,
    ) {
        self.stop_watch();
        let stop = Arc::new(AtomicBool::new(false));
        self.watch = Some(Watch {
            execution: execution.clone(),
            stop: Arc::clone(&stop),
        });
        let interval = if backend.is_demo() { 120 } else { 1_000 };
        let sender = self.watch_sender.clone();
        let context = context.clone();
        self.spawn(async move {
            let mut last: Option<String> = None;
            while !stop.load(Ordering::Relaxed) {
                let result = backend.execution(&session, &execution).await;
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let ended = match &result {
                    Ok(detail) => ended(detail.execution.status),
                    Err(_) => true,
                };
                let fingerprint = result
                    .as_ref()
                    .ok()
                    .and_then(|detail| serde_json::to_string(detail).ok());
                if fingerprint.is_none() || fingerprint != last {
                    last = fingerprint;
                    if sender.send((generation, result.map(Box::new))).is_err() {
                        break;
                    }
                    context.request_repaint();
                }
                if ended {
                    break;
                }
                clock::sleep(interval).await;
            }
        });
    }

    pub(crate) fn stop_watch(&mut self) {
        if let Some(watch) = self.watch.take() {
            watch.stop.store(true, Ordering::Relaxed);
        }
    }

    /// Watched states that arrived since the last frame. Never blocks.
    pub(crate) fn watch_events(&self) -> impl Iterator<Item = WatchEvent> + '_ {
        self.watch_receiver.try_iter()
    }
}

/// An execution in one of these states will not change again.
pub(crate) const fn ended(status: ExecutionStatus) -> bool {
    matches!(
        status,
        ExecutionStatus::Completed
            | ExecutionStatus::Failed
            | ExecutionStatus::Cancelled
            | ExecutionStatus::TimedOut
    )
}
