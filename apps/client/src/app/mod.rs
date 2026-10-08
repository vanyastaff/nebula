//! Application shell. Owns the workbench state and the effect runner, lays out the top bar, the
//! navigation and the page, turns the intents that views emitted into requests, and keeps the
//! execution watch on whatever run the visible page shows.

mod pages;

use crate::{
    api::Backend,
    demo::Demo,
    document::new_workflow_request,
    effects::{Effects, Operation},
    theme,
    transport::{Connection, SignIn},
    views::{
        Intent, Intents, catalog, connection, credentials, editor, executions, nav, runs, settings,
        shell, team, triggers, workflows,
    },
    widgets,
    workbench::{Page, Remembered, SignInMode, Workbench},
};
use eframe::egui;
use nebula_api_contract::v1::auth::{LoginRequest, SecretString};
use zeroize::Zeroizing;

/// Width of the navigation rail on wide windows.
const RAIL_WIDTH: f32 = 196.0;

/// Native and web workbench. The renderer never waits for network I/O.
pub struct ClientApp {
    workbench: Workbench,
    effects: Effects,
}

impl ClientApp {
    /// Create a workbench; native runtime construction is fallible.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, std::io::Error> {
        theme::install(&cc.egui_ctx);
        let mut workbench = Workbench::new(default_endpoint());
        if let Some(remembered) = cc
            .storage
            .and_then(|storage| eframe::get_value::<Remembered>(storage, REMEMBERED_KEY))
        {
            workbench.restore(usable(remembered));
        }
        Ok(Self {
            workbench,
            effects: Effects::new()?,
        })
    }

    /// Starts the request, or returns false when it cannot start: no server, or one already in
    /// flight. Callers change state for a request only after it started, so a dropped intent never
    /// leaves a "loading" or "uncertain" mark behind.
    fn dispatch(&mut self, context: &egui::Context, operation: Operation) -> bool {
        let Some(backend) = self.workbench.backend.clone() else {
            return false;
        };
        let writes = matches!(operation, Operation::Save(..) | Operation::Publish(..));
        let stamp = if writes {
            self.workbench.session.begin_write()
        } else {
            self.workbench.session.begin()
        };
        let Some(stamp) = stamp else {
            return false;
        };
        let session = self.workbench.session.context.clone();
        self.effects
            .start(context, stamp, backend, session, operation);
        true
    }

    fn open_workflow(&self) -> Option<String> {
        Some(self.workbench.session.draft()?.base.workflow.id.clone())
    }

    fn run_intent(&mut self, context: &egui::Context, intent: Intent) {
        // A busy session already has a request in flight; dropping the intent keeps local flags honest.
        if self.workbench.session.busy() {
            return;
        }
        match intent {
            Intent::SignIn => self.sign_in(context),
            Intent::OpenDemo => self.open_demo(context),
            Intent::OpenWorkspace => {
                if self.workbench.open_workspace() {
                    self.list_workflows(context, 1);
                }
            },
            Intent::ListWorkflows(page) => self.list_workflows(context, page),
            Intent::CreateWorkflow => {
                let request = new_workflow_request(&self.workbench.navigator.new_name);
                self.dispatch(context, Operation::Create(request));
            },
            Intent::LoadWorkflow(id) => {
                self.dispatch(context, Operation::Load(id));
            },
            Intent::SaveDraft => {
                let Some(draft) = self.workbench.session.draft() else {
                    return;
                };
                let request = draft.save_request();
                let id = draft.base.workflow.id.clone();
                if self.dispatch(context, Operation::Save(id, request))
                    && let Some(draft) = self.workbench.session.draft_mut()
                {
                    draft.uncertain_save = true;
                }
            },
            Intent::PublishDraft => {
                let Some(draft) = self.workbench.session.draft() else {
                    return;
                };
                let id = draft.base.workflow.id.clone();
                let revision = draft.base.revision;
                if self.dispatch(context, Operation::Publish(id, revision))
                    && let Some(draft) = self.workbench.session.draft_mut()
                {
                    draft.uncertain_save = true;
                }
            },
            Intent::RunDraft => {
                let Some(draft) = self.workbench.session.draft() else {
                    return;
                };
                let workflow = draft.base.workflow.id.clone();
                // A retry after an unknown outcome reuses the key, so the server can replay it.
                let key = draft
                    .start_key
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                if self.dispatch(context, Operation::Run(workflow, key.clone()))
                    && let Some(draft) = self.workbench.session.draft_mut()
                {
                    draft.start_key = Some(key);
                }
            },
            Intent::RefreshRuns => self.read_runs(context),
            Intent::LoadExecution(id) => {
                self.dispatch(context, Operation::Status(id));
            },
            Intent::LoadCatalog => {
                self.dispatch(context, Operation::Actions);
            },
            Intent::LoadSchema(action) => {
                if !self.workbench.schemas.contains_key(&action)
                    && self.dispatch(context, Operation::Action(action.clone()))
                {
                    self.workbench.begin_schema(&action);
                }
            },
            page => self.run_page_intent(context, page),
        }
    }

    fn list_workflows(&mut self, context: &egui::Context, page: usize) {
        if self.dispatch(context, Operation::List(page)) {
            self.workbench.navigator.read.begin();
        }
    }

    /// Reads the open workflow's recent runs and the chosen run.
    fn read_runs(&mut self, context: &egui::Context) {
        let Some(workflow) = self.open_workflow() else {
            return;
        };
        let execution = self
            .workbench
            .session
            .draft()
            .and_then(|draft| draft.execution_id.clone());
        if self.dispatch(context, Operation::History(workflow, execution)) {
            self.workbench.recent_runs_requested();
        }
    }

    /// Builds the sign-in request. A token leaves the form; a password stays until sign-in completes.
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
        self.workbench.begin_sign_in(Backend::Server(connection));
        self.dispatch(context, Operation::Connect(Some(intent)));
    }

    /// Opens a fresh demo workspace, which needs no server and no credentials.
    fn open_demo(&mut self, context: &egui::Context) {
        match Demo::new() {
            Ok(demo) => {
                self.workbench.begin_sign_in(Backend::Demo(demo));
                self.dispatch(context, Operation::Connect(None));
            },
            Err(error) => self.workbench.feedback.error(error.to_string()),
        }
    }

    fn receive(&mut self) {
        for (stamp, kind, result) in self.effects.completions() {
            self.workbench.receive(stamp, kind, result);
        }
        for (generation, result) in self.effects.watch_events() {
            self.workbench.receive_watch(generation, result);
        }
    }

    /// Streams the run the visible page shows while it can still change, and stops otherwise.
    fn reconcile_watch(&mut self, context: &egui::Context) {
        let wanted = self.workbench.wanted_watch();
        if wanted.as_deref() == self.effects.watched() {
            return;
        }
        match (
            wanted,
            self.workbench.backend.clone(),
            self.workbench.session.context.clone(),
        ) {
            (Some(execution), Some(backend), Some(session)) => self.effects.watch(
                context,
                self.workbench.session.generation(),
                backend,
                session,
                execution,
            ),
            _ => self.effects.stop_watch(),
        }
    }

    /// The editor: the node panel and the runs panel around the canvas.
    fn editor_page(&mut self, ui: &mut egui::Ui, intents: &mut Intents, wide: bool) {
        let workbench = &mut self.workbench;
        let has_draft = workbench.session.draft().is_some();
        if wide && has_draft {
            // The node sidebar slides in from the right over the full height, before the runs
            // panel claims the bottom. Dragging its edge shut closes it like the Close button.
            let mut open = editor::has_side_panel(workbench);
            let was_open = open;
            egui::Panel::right("side")
                .default_size(400.0)
                .size_range(320.0..=640.0)
                .resizable(true)
                .drag_to_open(false)
                .frame(theme::panel(theme::SIDEBAR))
                // The panel scrolls its own body, so its header and tabs stay in view.
                .show_collapsible(ui, &mut open, |ui| editor::side(ui, workbench, intents));
            if was_open && !open {
                editor::close_side_panel(workbench);
            }
            egui::Panel::bottom("runs")
                // Tall enough for a run's status, a node and its error without scrolling.
                .default_size(260.0)
                .size_range(230.0..=520.0)
                .resizable(true)
                .frame(theme::panel(theme::SIDEBAR))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("runs")
                        .show(ui, |ui| runs::show(ui, workbench, intents));
                });
        }
        egui::CentralPanel::default()
            .frame(theme::canvas())
            .show(ui, |ui| {
                if wide {
                    // The editor takes the whole page, so the canvas uses the height under its bar.
                    editor::show(ui, workbench, intents, false);
                    return;
                }
                egui::ScrollArea::vertical()
                    .id_salt("editor-page")
                    .show(ui, |ui| {
                        editor::show(ui, workbench, intents, true);
                        if editor::has_side_panel(workbench) {
                            ui.add_space(theme::SPACE_MD);
                            theme::card_block(ui, |ui| editor::side(ui, workbench, intents));
                        }
                        if has_draft {
                            ui.add_space(theme::SPACE_MD);
                            theme::card_block(ui, |ui| runs::show(ui, workbench, intents));
                        }
                    });
            });
    }
}

impl eframe::App for ClientApp {
    /// Keeps the server address, email, sign-in mode and recent workspaces; never a secret.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, REMEMBERED_KEY, &self.workbench.remembered());
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.receive();
        if self.workbench.navigator.take_refresh() {
            self.list_workflows(ui.ctx(), 1);
        }
        let wide = ui.available_width() >= theme::WIDE_LAYOUT_MIN;
        let workspace = self.workbench.workspace_open() && !self.workbench.workspace_form_open;
        let mut intents = Intents::new();

        if workspace {
            nav::shortcuts(ui.ctx(), &mut self.workbench);
        }
        egui::Panel::top("header")
            .frame(theme::bar())
            .show(ui, |ui| shell::header(ui, &mut self.workbench, wide));
        if workspace {
            if wide {
                egui::Panel::left("navigation")
                    .exact_size(RAIL_WIDTH)
                    .resizable(false)
                    .frame(theme::panel(theme::SIDEBAR))
                    .show(ui, |ui| nav::rail(ui, &mut self.workbench));
            } else {
                egui::Panel::top("pages")
                    .frame(theme::bar())
                    .show(ui, |ui| nav::tabs(ui, &mut self.workbench));
            }
        }
        if workspace && self.workbench.page == Page::Editor {
            self.editor_page(ui, &mut intents, wide);
        } else {
            let workbench = &mut self.workbench;
            egui::CentralPanel::default()
                .frame(theme::canvas())
                .show(ui, |ui| {
                    // The page scrolls at the window's edge, clear of the centred column.
                    egui::ScrollArea::vertical()
                        .id_salt(("page", workbench.page as u8, workspace))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if !workspace {
                                connection::show(ui, workbench, &mut intents);
                                return;
                            }
                            widgets::page_column(ui, theme::PAGE_MAX_WIDTH, |ui| {
                                ui.add_space(theme::SPACE_LG);
                                match workbench.page {
                                    Page::Workflows | Page::Editor => {
                                        workflows::show(ui, workbench, &mut intents);
                                    },
                                    Page::Executions => {
                                        executions::show(ui, workbench, &mut intents);
                                    },
                                    Page::Catalog => catalog::show(ui, workbench, &mut intents),
                                    Page::Triggers => triggers::show(ui, workbench, &mut intents),
                                    Page::Credentials => {
                                        credentials::show(ui, workbench, &mut intents);
                                    },
                                    Page::Team => team::show(ui, workbench, &mut intents),
                                    Page::Settings => settings::show(ui, workbench, &mut intents),
                                }
                                ui.add_space(theme::SPACE_XL);
                            });
                        });
                });
        }

        nav::sheet(ui.ctx(), &mut self.workbench);
        shell::toast(ui.ctx(), &mut self.workbench);

        for intent in intents {
            self.run_intent(ui.ctx(), intent);
        }
        self.reconcile_watch(ui.ctx());
    }
}

const REMEMBERED_KEY: &str = "nebula-client-remembered";

/// The browser build signs in against the page's own origin, so a remembered address is not reused.
#[cfg(target_arch = "wasm32")]
fn usable(mut remembered: Remembered) -> Remembered {
    remembered.endpoint.clear();
    remembered
}

#[cfg(not(target_arch = "wasm32"))]
const fn usable(remembered: Remembered) -> Remembered {
    remembered
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
