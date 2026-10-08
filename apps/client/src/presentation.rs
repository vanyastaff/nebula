//! Egui presentation and host scheduling. Session/draft invariants live below this module.

use crate::{
    document::Draft,
    session::{DraftKey, RequestStamp, Session, SessionContext},
    theme,
    transport::{Connection, Failure, SignIn, SignedIn},
};
use eframe::egui;
use nebula_api_contract::v1::{
    auth::{LoginRequest, SecretString},
    execution::{
        ExecutionDetailResponse, ExecutionNodeOutput, ExecutionResponse, ExecutionStatus,
        ListExecutionsResponse,
    },
    me::MeResponse,
    workflow::{
        ListWorkflowsResponse, UpdateWorkflowDocumentRequest, WorkflowDocumentResponse,
        WorkflowResponse,
    },
};
use std::sync::mpsc;
use zeroize::{Zeroize, Zeroizing};

enum Operation {
    Connect(SignIn),
    List(usize),
    Load(String),
    Save(String, UpdateWorkflowDocumentRequest),
    Publish(String, u64),
    Run(String, String),
    History(String),
    Status(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RequestKind {
    Read,
    Save,
    Publish,
    Run,
    Connect,
}

impl Operation {
    fn kind(&self) -> RequestKind {
        match self {
            Self::Connect(_) => RequestKind::Connect,
            Self::Save(..) => RequestKind::Save,
            Self::Publish(..) => RequestKind::Publish,
            Self::Run(..) => RequestKind::Run,
            _ => RequestKind::Read,
        }
    }
}

enum Reply {
    Connected(SignedIn),
    Listed(ListWorkflowsResponse),
    Loaded(WorkflowDocumentResponse),
    Saved(WorkflowDocumentResponse),
    Published(WorkflowDocumentResponse),
    Started(ExecutionResponse),
    History(ListExecutionsResponse),
    Status(Box<ExecutionDetailResponse>),
}

async fn perform(
    connection: Connection,
    context: Option<SessionContext>,
    operation: Operation,
) -> Result<Reply, Failure> {
    if let Operation::Connect(intent) = operation {
        return connection.sign_in(intent).await.map(Reply::Connected);
    }
    let context = context.ok_or(Failure::Configuration)?;
    let org = context.organization.as_str();
    let workspace = context.workspace_selector.as_str();
    match operation {
        Operation::Connect(_) => Err(Failure::Configuration),
        Operation::List(page) => connection
            .list(org, workspace, page)
            .await
            .map(Reply::Listed),
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
        Operation::History(id) => connection
            .history(org, workspace, &id)
            .await
            .map(Reply::History),
        Operation::Status(id) => connection
            .status(org, workspace, &id)
            .await
            .map(|detail| Reply::Status(Box::new(detail))),
    }
}

/// Native and web workbench. The renderer never waits for network I/O.
pub struct ClientApp {
    session: Session,
    connection: Option<Connection>,
    profile: Option<MeResponse>,
    endpoint: String,
    email: String,
    password: String,
    totp: String,
    token: String,
    use_token: bool,
    organization: String,
    workspace: String,
    workflows: Vec<WorkflowResponse>,
    page: usize,
    total: usize,
    selected_node: String,
    selected_parameter: String,
    parameter_text: String,
    status: Option<Box<ExecutionDetailResponse>>,
    history: Option<ListExecutionsResponse>,
    message: String,
    failure: bool,
    show_connection: bool,
    tx: mpsc::Sender<(RequestStamp, RequestKind, Result<Reply, Failure>)>,
    rx: mpsc::Receiver<(RequestStamp, RequestKind, Result<Reply, Failure>)>,
    #[cfg(not(target_arch = "wasm32"))]
    runtime: tokio::runtime::Runtime,
}

impl ClientApp {
    /// Create a workbench; native runtime construction is fallible.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, std::io::Error> {
        theme::install(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel();
        #[cfg(target_arch = "wasm32")]
        let endpoint = web_sys::window()
            .and_then(|window| window.location().origin().ok())
            .unwrap_or_default();
        #[cfg(not(target_arch = "wasm32"))]
        let endpoint = "http://127.0.0.1:8080".to_owned();
        Ok(Self {
            session: Session::default(),
            connection: None,
            profile: None,
            endpoint,
            email: String::new(),
            password: String::new(),
            totp: String::new(),
            token: String::new(),
            use_token: false,
            organization: String::new(),
            workspace: String::new(),
            workflows: Vec::new(),
            page: 1,
            total: 0,
            selected_node: String::new(),
            selected_parameter: String::new(),
            parameter_text: String::new(),
            status: None,
            history: None,
            message: "Connect to an existing Nebula server to open a workspace.".into(),
            failure: false,
            show_connection: false,
            tx,
            rx,
            #[cfg(not(target_arch = "wasm32"))]
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?,
        })
    }

    fn dispatch(&mut self, context: &egui::Context, operation: Operation) {
        let Some(connection) = self.connection.clone() else {
            return;
        };
        let Some(stamp) = self.session.begin() else {
            return;
        };
        self.message = "Waiting for the server…".into();
        self.failure = false;
        let workspace = self.session.context.clone();
        let sender = self.tx.clone();
        let context = context.clone();
        let kind = operation.kind();
        let work = async move {
            let result = perform(connection, workspace, operation).await;
            let _ = sender.send((stamp, kind, result));
            context.request_repaint();
        };
        #[cfg(not(target_arch = "wasm32"))]
        self.runtime.spawn(work);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(work);
    }

    fn clear_selection(&mut self) {
        self.selected_node.clear();
        self.selected_parameter.clear();
        self.parameter_text.zeroize();
        self.status = None;
        self.history = None;
    }

    fn disconnect(&mut self) {
        self.session.switch(None);
        self.connection = None;
        self.profile = None;
        self.workflows.clear();
        self.password.zeroize();
        self.token.zeroize();
        self.totp.zeroize();
        self.clear_selection();
        self.message = "Disconnected. Your drafts remain available in this app session.".into();
        self.failure = false;
    }

    fn receive(&mut self) {
        while let Ok((stamp, kind, reply)) = self.rx.try_recv() {
            if !self.session.accept(stamp) {
                continue;
            }
            match reply {
                Err(error) => {
                    self.message = error.to_string();
                    self.failure = true;
                    // A definitive conflict leaves the draft intact. Reads never discard it.
                    if let Some(draft) = self.session.draft_mut() {
                        if matches!(kind, RequestKind::Save | RequestKind::Publish)
                            && error != Failure::OutcomeUnknown
                        {
                            draft.uncertain_save = false;
                            draft.save_conflict = error == Failure::Conflict;
                        }
                        if kind == RequestKind::Run && error != Failure::OutcomeUnknown {
                            draft.start_key = None;
                        }
                    }
                    if error == Failure::Unauthorized {
                        self.disconnect();
                        self.message = error.to_string();
                        self.failure = true;
                    }
                },
                Ok(Reply::Connected(signed_in)) => {
                    self.connection = Some(signed_in.connection);
                    self.profile = Some(signed_in.profile);
                    self.message =
                        "Signed in. Enter your organization and workspace slug or ID.".into();
                },
                Ok(Reply::Listed(page)) => {
                    self.workflows = page.workflows;
                    self.total = page.total;
                    self.page = page.page;
                    self.message = format!("{} workflows in this workspace.", self.total);
                },
                Ok(Reply::Loaded(document)) => {
                    let Some(key) = self.session.selected.clone() else {
                        continue;
                    };
                    if document.workflow.id != key.workflow {
                        self.message = Failure::InvalidResponse.to_string();
                        self.failure = true;
                        continue;
                    }
                    if let Some(draft) = self.session.drafts.get_mut(&key) {
                        if draft.dirty() || draft.uncertain_save {
                            draft.remote = Some(document);
                            self.message = "Server version read. Review it before replacing or reapplying your draft.".into();
                        } else {
                            draft.saved(document);
                            self.message = "Workflow loaded.".into();
                        }
                    } else {
                        match Draft::new(document) {
                            Ok(draft) => {
                                self.session.drafts.insert(key, draft);
                                self.message = "Workflow loaded.".into();
                            },
                            Err(error) => {
                                self.message = error.to_string();
                                self.failure = true;
                            },
                        }
                    }
                    self.selected_parameter.clear();
                    self.parameter_text.zeroize();
                },
                Ok(Reply::Saved(document)) => {
                    if let Some(draft) = self.session.draft_mut() {
                        draft.saved(document);
                    }
                    self.message = "Changes saved. Publish this version before running it.".into();
                },
                Ok(Reply::Published(document)) => {
                    if let Some(draft) = self.session.draft_mut() {
                        if draft.definition["nodes"] == document.definition["nodes"] {
                            draft.saved(document);
                            self.message =
                                "Workflow published. Run uses the server's current publication."
                                    .into();
                        } else {
                            draft.remote = Some(document);
                            self.message = "The workflow changed during publication. Review the server version.".into();
                            self.failure = true;
                        }
                    }
                },
                Ok(Reply::Started(receipt)) => {
                    if let Some(draft) = self.session.draft_mut() {
                        if receipt.workflow_id != draft.base.workflow.id {
                            self.message = Failure::OutcomeUnknown.to_string();
                            self.failure = true;
                            continue;
                        }
                        draft.execution_id = Some(receipt.id);
                        draft.start_key = None;
                    }
                    self.status = None;
                    self.message =
                        "Run accepted. Read persisted status to see whether it has started.".into();
                },
                Ok(Reply::History(history)) => {
                    self.history = Some(history);
                    self.message = "Recent runs read from the server.".into();
                },
                Ok(Reply::Status(status)) => {
                    self.status = Some(status);
                    self.message = "Execution snapshot read from persisted server state.".into();
                },
            }
        }
    }

    fn connection_ui(&mut self, ui: &mut egui::Ui) {
        if self.profile.is_none() {
            ui.heading("Connect to Nebula");
            ui.label("Open your workflows on an existing server.");
            ui.add_space(12.0);
            ui.add_enabled_ui(!self.session.busy(), |ui| {
                ui.label("Server address");
                ui.add(theme::field(&mut self.endpoint));
                ui.checkbox(&mut self.use_token, "Use a personal access token");
                if self.use_token {
                    ui.label("Personal access token");
                    ui.add(theme::field(&mut self.token).password(true));
                } else {
                    ui.label("Email");
                    ui.add(theme::field(&mut self.email));
                    ui.label("Password");
                    ui.add(theme::field(&mut self.password).password(true));
                    ui.label("Authenticator code (optional)");
                    ui.add(theme::field(&mut self.totp).password(true));
                }
                ui.add_space(8.0);
                if ui
                    .add_sized([ui.available_width(), 40.0], theme::primary("Sign in"))
                    .clicked()
                {
                    match Connection::new(&self.endpoint) {
                        Ok(connection) => {
                            self.session.switch(None);
                            self.connection = Some(connection);
                            let intent = if self.use_token {
                                SignIn::Token(Zeroizing::new(std::mem::take(&mut self.token)))
                            } else {
                                SignIn::Password(LoginRequest {
                                    email: self.email.clone(),
                                    password: SecretString::new(std::mem::take(&mut self.password)),
                                    totp: (!self.totp.is_empty())
                                        .then(|| std::mem::take(&mut self.totp)),
                                })
                            };
                            self.dispatch(ui.ctx(), Operation::Connect(intent));
                        },
                        Err(error) => {
                            self.message = error.to_string();
                            self.failure = true;
                        },
                    }
                }
            });
        } else {
            ui.heading("Open a workspace");
            ui.label("Enter the organization and workspace provided by your server.");
            ui.add_space(12.0);
            ui.add_enabled_ui(!self.session.busy(), |ui| {
                ui.label("Organization");
                ui.add(theme::field(&mut self.organization));
                ui.label("Workspace");
                ui.add(theme::field(&mut self.workspace));
                if ui
                    .add_sized(
                        [ui.available_width(), 40.0],
                        theme::primary("Open workspace"),
                    )
                    .clicked()
                    && let (Some(connection), Some(profile)) = (&self.connection, &self.profile)
                {
                    self.session.switch(Some(SessionContext {
                        endpoint: connection.endpoint().into(),
                        principal: profile.user_id.clone(),
                        organization: self.organization.trim().into(),
                        workspace_selector: self.workspace.trim().into(),
                    }));
                    self.workflows.clear();
                    self.clear_selection();
                    self.page = 1;
                    self.show_connection = false;
                    self.dispatch(ui.ctx(), Operation::List(1));
                }
            });
        }
    }

    fn header_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Nebula").size(23.0).strong());
            if let Some(context) = &self.session.context {
                ui.colored_label(
                    theme::MUTED,
                    format!("{} / {}", context.organization, context.workspace_selector),
                );
            } else {
                ui.colored_label(theme::MUTED, "Workflow editor");
            }
            if self.profile.is_some() {
                if ui.button("Workspace…").clicked() {
                    self.show_connection = !self.show_connection;
                }
                if ui.button("Disconnect").clicked() {
                    self.disconnect();
                    self.show_connection = false;
                }
            }
        });
    }

    fn feedback_ui(&self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if self.session.busy() {
                ui.spinner();
            }
            let color = if self.failure {
                theme::ERROR
            } else {
                theme::MUTED
            };
            ui.label(egui::RichText::new(&self.message).color(color).size(14.0));
        });
    }

    fn workflow_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Workflows");
        theme::caption(ui, format!("{} in this workspace", self.total));
        if self.session.context.is_none() {
            ui.label("Open a workspace to list its workflows.");
            return;
        }
        ui.add_enabled_ui(!self.session.busy(), |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Refresh").clicked() {
                    self.dispatch(ui.ctx(), Operation::List(self.page));
                }
                if ui
                    .add_enabled(self.page > 1, egui::Button::new("Previous"))
                    .clicked()
                {
                    self.dispatch(ui.ctx(), Operation::List(self.page - 1));
                }
                if ui
                    .add_enabled(self.page * 25 < self.total, egui::Button::new("Next"))
                    .clicked()
                {
                    self.dispatch(ui.ctx(), Operation::List(self.page + 1));
                }
            });
            for workflow in self.workflows.clone() {
                let selected = self
                    .session
                    .selected
                    .as_ref()
                    .is_some_and(|key| key.workflow == workflow.id);
                if ui
                    .add_sized(
                        [ui.available_width(), 40.0],
                        egui::Button::new(&workflow.name).selected(selected),
                    )
                    .clicked()
                    && let Some(context) = self.session.context.clone()
                {
                    self.session.selected = Some(DraftKey {
                        context,
                        workflow: workflow.id.clone(),
                    });
                    self.clear_selection();
                    self.dispatch(ui.ctx(), Operation::Load(workflow.id));
                }
            }
        });
        ui.separator();
        theme::caption(
            ui,
            "Drafts stay here when you disconnect. Closing the app clears them.",
        );
    }

    fn editor_ui(&mut self, ui: &mut egui::Ui) {
        let Some(draft) = self.session.draft() else {
            ui.add_space(42.0);
            ui.heading("Choose a workflow");
            ui.label(
                "Select a workflow from the sidebar to inspect its nodes and edit parameters.",
            );
            return;
        };
        ui.heading(&draft.base.workflow.name);
        theme::caption(
            ui,
            format!(
                "Revision {}{}",
                draft.base.revision,
                if draft.dirty() {
                    " • Unsaved changes"
                } else {
                    " • Saved"
                }
            ),
        );
        let id = draft.base.workflow.id.clone();
        let revision = draft.base.revision;
        let can_save = draft.dirty()
            && draft.remote.is_none()
            && !draft.uncertain_save
            && !draft.save_conflict;
        let can_publish = !draft.dirty()
            && draft.remote.is_none()
            && !draft.uncertain_save
            && !draft.save_conflict;
        ui.add_enabled_ui(!self.session.busy(), |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Undo").clicked()
                    && let Some(draft) = self.session.draft_mut()
                {
                    let _ = draft.undo();
                    self.selected_parameter.clear();
                }
                if ui.button("Redo").clicked()
                    && let Some(draft) = self.session.draft_mut()
                {
                    let _ = draft.redo();
                    self.selected_parameter.clear();
                }
                if ui
                    .add_enabled(can_save, theme::primary("Save changes"))
                    .clicked()
                    && let Some(draft) = self.session.draft_mut()
                {
                    let request = draft.save_request();
                    draft.uncertain_save = true;
                    self.dispatch(ui.ctx(), Operation::Save(id.clone(), request));
                }
                if ui
                    .add_enabled(can_publish, egui::Button::new("Publish"))
                    .clicked()
                {
                    if let Some(draft) = self.session.draft_mut() {
                        draft.uncertain_save = true;
                    }
                    self.dispatch(ui.ctx(), Operation::Publish(id.clone(), revision));
                }
                if ui.button("Read server version").clicked() {
                    self.dispatch(ui.ctx(), Operation::Load(id.clone()));
                }
            });
        });
        if let Some(remote) = self.session.draft().and_then(|draft| draft.remote.as_ref()) {
            ui.colored_label(
                theme::WARNING,
                format!("Review server revision {}", remote.revision),
            );
            egui::CollapsingHeader::new("Server nodes for comparison").show(ui, |ui| {
                ui.label(
                    serde_json::to_string_pretty(&remote.definition["nodes"]).unwrap_or_default(),
                );
            });
            ui.label("Reapply overwrites only the parameters you edited, including parameters changed on the server. Other server changes are preserved.");
            ui.add_enabled_ui(!self.session.busy(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Reapply my parameter edits").clicked()
                        && let Some(draft) = self.session.draft_mut()
                    {
                        match draft.reapply() {
                            Ok(()) => {
                                self.message = "Draft rebased. Review and save changes.".into();
                            },
                            Err(error) => {
                                self.message = error.to_string();
                                self.failure = true;
                            },
                        }
                        self.selected_parameter.clear();
                    }
                    if ui.button("Discard draft and use server version").clicked()
                        && let Some(draft) = self.session.draft_mut()
                        && let Some(remote) = draft.remote.take()
                    {
                        draft.saved(remote);
                        self.selected_parameter.clear();
                    }
                });
            });
        }
        ui.separator();
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Workflow nodes").strong().size(18.0));
        let nodes = self
            .session
            .draft()
            .and_then(|draft| draft.definition["nodes"].as_array())
            .cloned()
            .unwrap_or_default();
        ui.add_enabled_ui(!self.session.busy(), |ui| {
            for node in nodes {
                let node_id = node["id"].as_str().unwrap_or_default();
                egui::CollapsingHeader::new(format!(
                    "{}  /  {}",
                    node["name"].as_str().unwrap_or(node_id),
                    node["action_key"].as_str().unwrap_or_default()
                ))
                .default_open(true)
                .show(ui, |ui| {
                    let Some(parameters) = node["parameters"].as_object() else {
                        ui.label("This node has no configured parameters.");
                        return;
                    };
                    for (name, value) in parameters {
                        let literal = value["type"].as_str() == Some("literal");
                        ui.horizontal_wrapped(|ui| {
                            let selected =
                                self.selected_node == node_id && self.selected_parameter == *name;
                            if ui
                                .add_enabled(
                                    literal,
                                    egui::Button::new(name)
                                        .selected(selected)
                                        .min_size(egui::vec2(120.0, 34.0)),
                                )
                                .clicked()
                            {
                                self.selected_node = node_id.into();
                                self.selected_parameter.clone_from(name);
                                self.parameter_text = serde_json::to_string_pretty(&value["value"])
                                    .unwrap_or_default();
                            }
                            if literal {
                                let preview = value["value"].to_string();
                                let preview: String = preview.chars().take(64).collect();
                                theme::caption(ui, preview);
                            }
                        });
                        if !literal {
                            ui.label(format!(
                                "{} parameter (read only in this slice)",
                                value["type"].as_str().unwrap_or("Unknown")
                            ));
                        }
                    }
                });
            }
            if !self.selected_parameter.is_empty() {
                ui.add_space(14.0);
                ui.separator();
                ui.heading(&self.selected_parameter);
                theme::caption(ui, "Edit the JSON value, then apply it to your draft.");
                ui.add(
                    egui::TextEdit::multiline(&mut self.parameter_text)
                        .code_editor()
                        .desired_rows(8)
                        .desired_width(f32::INFINITY),
                );
                if ui.add(theme::primary("Apply parameter edit")).clicked()
                    && let Some(draft) = self.session.draft_mut()
                {
                    match draft.edit(
                        &self.selected_node,
                        &self.selected_parameter,
                        &self.parameter_text,
                    ) {
                        Ok(()) => {
                            self.message = "Parameter edited in your draft.".into();
                            self.failure = false;
                        },
                        Err(error) => {
                            self.message = error.to_string();
                            self.failure = true;
                        },
                    }
                }
            }
        });
    }

    fn execution_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Runs").size(20.0).strong());
        let Some(draft) = self.session.draft() else {
            ui.label("Open a workflow to inspect recent runs.");
            return;
        };
        let workflow = draft.base.workflow.id.clone();
        let execution = draft.execution_id.clone();
        let can_run = !draft.dirty()
            && draft.remote.is_none()
            && !draft.uncertain_save
            && !draft.save_conflict;
        let pending = draft.start_key.is_some();
        theme::caption(
            ui,
            "Runs the published workflow on the server. Save and publish your changes first.",
        );
        ui.add_enabled_ui(!self.session.busy(), |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        can_run || pending,
                        theme::primary(if pending {
                            "Reconcile pending run"
                        } else {
                            "Run workflow"
                        }),
                    )
                    .clicked()
                    && let Some(draft) = self.session.draft_mut()
                {
                    let key = draft
                        .start_key
                        .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
                        .clone();
                    self.dispatch(ui.ctx(), Operation::Run(workflow.clone(), key));
                }
                if ui.button("Recent runs").clicked() {
                    self.dispatch(ui.ctx(), Operation::History(workflow.clone()));
                }
                if let Some(id) = execution
                    && ui.button("Refresh status").clicked()
                {
                    self.dispatch(ui.ctx(), Operation::Status(id));
                }
            });
            if let Some(history) = &self.history {
                let mut selected = None;
                egui::CollapsingHeader::new("Run history")
                    .default_open(self.status.is_none())
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("history")
                            .max_height(110.0)
                            .show(ui, |ui| {
                                for execution in &history.items {
                                    if ui
                                        .button(format!(
                                            "{:?}    {}",
                                            execution.status,
                                            execution
                                                .created_at
                                                .get(..19)
                                                .unwrap_or(&execution.created_at)
                                                .replace('T', " ")
                                        ))
                                        .clicked()
                                    {
                                        selected = Some(execution.id.clone());
                                    }
                                }
                            });
                    });
                if let Some(id) = selected {
                    if let Some(draft) = self.session.draft_mut() {
                        draft.execution_id = Some(id.clone());
                    }
                    self.dispatch(ui.ctx(), Operation::Status(id));
                }
            }
        });
        if let Some(status) = &self.status {
            ui.separator();
            let color = match status.execution.status {
                ExecutionStatus::Completed => theme::SUCCESS,
                ExecutionStatus::Failed | ExecutionStatus::TimedOut => theme::ERROR,
                _ => theme::ACCENT,
            };
            ui.label(
                egui::RichText::new(format!("{:?}", status.execution.status))
                    .size(20.0)
                    .strong()
                    .color(color),
            );
            for (name, node) in &status.nodes {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(name).strong());
                    theme::caption(ui, format!("{:?}", node.status));
                    if let Some(ExecutionNodeOutput::Inline { value }) = &node.output {
                        ui.monospace(value.to_string());
                    }
                });
            }
            ui.collapsing("Execution details", |ui| {
                theme::caption(ui, &status.execution.id);
                theme::caption(ui, format!("Server snapshot {}", status.snapshot_version));
                theme::caption(ui, format!("Updated {}", status.execution.updated_at));
                theme::caption(
                    ui,
                    format!(
                        "Started {}",
                        status
                            .execution
                            .started_at
                            .as_deref()
                            .unwrap_or("Not started")
                    ),
                );
                theme::caption(
                    ui,
                    format!(
                        "Finished {}",
                        status
                            .execution
                            .finished_at
                            .as_deref()
                            .unwrap_or("Not terminal")
                    ),
                );
            });
        }
    }
}

impl eframe::App for ClientApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.receive();
        ui.set_style(ui.ctx().global_style());
        let wide = ui.available_width() >= 760.0;
        let connected = self.session.context.is_some() && !self.show_connection;
        egui::Panel::top("header")
            .frame(theme::panel(theme::NAVIGATION))
            .show(ui, |ui| {
                self.header_ui(ui);
                self.feedback_ui(ui);
            });
        if connected && wide {
            egui::Panel::left("workflows")
                .default_size(240.0)
                .size_range(190.0..=320.0)
                .resizable(true)
                .frame(theme::panel(theme::NAVIGATION))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("navigation")
                        .show(ui, |ui| self.workflow_list(ui));
                });
            if self.session.draft().is_some() {
                egui::Panel::bottom("runs")
                    .default_size(300.0)
                    .size_range(250.0..=440.0)
                    .resizable(true)
                    .frame(theme::panel(theme::NAVIGATION))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("runs")
                            .show(ui, |ui| self.execution_ui(ui));
                    });
            }
        }
        egui::CentralPanel::default()
            .frame(theme::panel(if connected {
                egui::Color32::WHITE
            } else {
                theme::BACKGROUND
            }))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("document")
                    .show(ui, |ui| {
                        if connected {
                            if !wide {
                                ui.collapsing("Workflows", |ui| self.workflow_list(ui));
                            }
                            self.editor_ui(ui);
                            if !wide {
                                ui.separator();
                                self.execution_ui(ui);
                            }
                        } else {
                            ui.add_space(36.0);
                            let inset = ((ui.available_width() - 480.0) / 2.0).max(0.0);
                            ui.horizontal(|ui| {
                                ui.add_space(inset);
                                egui::Frame::new()
                                    .fill(egui::Color32::WHITE)
                                    .inner_margin(26)
                                    .corner_radius(10)
                                    .show(ui, |ui| {
                                        ui.with_layout(
                                            egui::Layout::top_down(egui::Align::Min),
                                            |ui| {
                                                ui.set_width(
                                                    (ui.available_width() - 52.0)
                                                        .clamp(180.0, 428.0),
                                                );
                                                self.connection_ui(ui);
                                            },
                                        );
                                    });
                            });
                        }
                    });
            });
    }
}

impl Drop for ClientApp {
    fn drop(&mut self) {
        self.password.zeroize();
        self.token.zeroize();
        self.totp.zeroize();
        self.parameter_text.zeroize();
    }
}
