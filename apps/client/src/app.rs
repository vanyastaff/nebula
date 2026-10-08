//! Application shell. Owns the workbench state and the effect runner, lays out the panels and
//! turns the intents that views emitted into network requests.
use crate::{
    document::new_workflow_request,
    effects::{Effects, Operation},
    theme,
    transport::{Connection, SignIn},
    views::{Intent, Intents, connection, editor, navigator, runs, shell},
    widgets,
    workbench::{SignInMode, Workbench},
};
use eframe::egui;
use nebula_api_contract::v1::auth::{LoginRequest, SecretString};
use zeroize::Zeroizing;

/// Native and web workbench. The renderer never waits for network I/O.
pub struct ClientApp {
    workbench: Workbench,
    effects: Effects,
}

impl ClientApp {
    /// Create a workbench; native runtime construction is fallible.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, std::io::Error> {
        theme::install(&cc.egui_ctx);
        Ok(Self {
            workbench: Workbench::new(default_endpoint()),
            effects: Effects::new()?,
        })
    }

    fn dispatch(&mut self, context: &egui::Context, operation: Operation) {
        let Some(connection) = self.workbench.connection.clone() else {
            return;
        };
        let Some(stamp) = self.workbench.session.begin() else {
            return;
        };
        let session = self.workbench.session.context.clone();
        self.effects
            .start(context, stamp, connection, session, operation);
    }

    fn run_intent(&mut self, context: &egui::Context, intent: Intent) {
        // A busy session already has a request in flight; dropping the intent keeps local flags honest.
        if self.workbench.session.busy() {
            return;
        }
        match intent {
            Intent::SignIn => self.sign_in(context),
            Intent::OpenWorkspace => {
                if self.workbench.open_workspace() {
                    self.dispatch(context, Operation::List(1));
                }
            },
            Intent::ListWorkflows(page) => self.dispatch(context, Operation::List(page)),
            Intent::CreateWorkflow => {
                let request = new_workflow_request(&self.workbench.navigator.new_name);
                self.dispatch(context, Operation::Create(request));
            },
            Intent::LoadWorkflow(id) => self.dispatch(context, Operation::Load(id)),
            Intent::SaveDraft => {
                let Some(draft) = self.workbench.session.draft_mut() else {
                    return;
                };
                let request = draft.save_request();
                draft.uncertain_save = true;
                let id = draft.base.workflow.id.clone();
                self.dispatch(context, Operation::Save(id, request));
            },
            Intent::PublishDraft => {
                let Some(draft) = self.workbench.session.draft_mut() else {
                    return;
                };
                draft.uncertain_save = true;
                let id = draft.base.workflow.id.clone();
                let revision = draft.base.revision;
                self.dispatch(context, Operation::Publish(id, revision));
            },
            Intent::RunDraft => {
                let Some(draft) = self.workbench.session.draft_mut() else {
                    return;
                };
                let workflow = draft.base.workflow.id.clone();
                let key = draft
                    .start_key
                    .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
                    .clone();
                self.dispatch(context, Operation::Run(workflow, key));
            },
            Intent::LoadRecentRuns => {
                let Some(draft) = self.workbench.session.draft() else {
                    return;
                };
                let id = draft.base.workflow.id.clone();
                self.dispatch(context, Operation::History(id));
            },
            Intent::LoadExecution(id) => self.dispatch(context, Operation::Status(id)),
            Intent::LoadCatalog => self.dispatch(context, Operation::Actions),
        }
    }

    /// Moves the sign-in secrets out of the form before the request starts.
    fn sign_in(&mut self, context: &egui::Context) {
        let form = &mut self.workbench.form;
        let connection = match Connection::new(&form.endpoint) {
            Ok(connection) => connection,
            Err(error) => {
                self.workbench.feedback.error(error.to_string());
                return;
            },
        };
        let intent = match form.mode {
            SignInMode::Token => SignIn::Token(Zeroizing::new(std::mem::take(&mut form.token))),
            // The password and code stay in the form until sign-in completes, so a second-factor retry
            // does not ask for the password again. `clear_secrets` wipes them on success or failure.
            SignInMode::Password => SignIn::Password(LoginRequest {
                email: form.email.clone(),
                password: SecretString::new(form.password.clone()),
                totp: (!form.totp.is_empty()).then(|| form.totp.clone()),
            }),
        };
        self.workbench.begin_sign_in(connection);
        self.dispatch(context, Operation::Connect(intent));
    }

    fn receive(&mut self) {
        for (stamp, kind, result) in self.effects.completions() {
            self.workbench.receive(stamp, kind, result);
        }
    }
}

impl eframe::App for ClientApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.receive();
        if self.workbench.navigator.take_refresh() {
            self.dispatch(ui.ctx(), Operation::List(1));
        }
        let wide = ui.available_width() >= theme::WIDE_LAYOUT_MIN;
        let workspace = self.workbench.workspace_open() && !self.workbench.workspace_form_open;
        let has_draft = self.workbench.session.draft().is_some();
        let mut intents = Intents::new();
        let workbench = &mut self.workbench;

        egui::Panel::top("header")
            .frame(theme::bar())
            .show(ui, |ui| shell::header(ui, workbench, wide));
        if workspace && wide {
            egui::Panel::left("sidebar")
                .default_size(260.0)
                .size_range(220.0..=340.0)
                .resizable(true)
                .frame(theme::panel(theme::SIDEBAR))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("sidebar")
                        .show(ui, |ui| navigator::show(ui, workbench, &mut intents));
                });
            if has_draft {
                egui::Panel::bottom("runs")
                    .default_size(300.0)
                    .size_range(220.0..=440.0)
                    .resizable(true)
                    .frame(theme::panel(theme::SIDEBAR))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("runs")
                            .show(ui, |ui| runs::show(ui, workbench, &mut intents));
                    });
            }
        }
        egui::CentralPanel::default()
            .frame(theme::canvas())
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("document")
                    .show(ui, |ui| {
                        if !workspace {
                            connection::show(ui, workbench, &mut intents);
                        } else if !wide && workbench.sidebar_open {
                            // Narrow layouts swap the page for the workflow list until one is opened.
                            widgets::page_column(ui, theme::PAGE_MAX_WIDTH, |ui| {
                                theme::card_block(ui, |ui| {
                                    navigator::show(ui, workbench, &mut intents);
                                });
                            });
                        } else {
                            widgets::page_column(ui, theme::PAGE_MAX_WIDTH, |ui| {
                                theme::card_block(ui, |ui| {
                                    editor::show(ui, workbench, &mut intents);
                                });
                                if !wide && has_draft {
                                    ui.add_space(theme::SPACE_MD);
                                    theme::card_block(ui, |ui| {
                                        runs::show(ui, workbench, &mut intents);
                                    });
                                }
                            });
                        }
                    });
            });

        shell::toast(ui.ctx(), workbench);

        for intent in intents {
            self.run_intent(ui.ctx(), intent);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn default_endpoint() -> String {
    web_sys::window()
        .and_then(|window| window.location().origin().ok())
        .unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
fn default_endpoint() -> String {
    "http://127.0.0.1:8080".to_owned()
}
