//! The actions this server's release can run, grouped by plugin. Choosing one shows its
//! description and its parameter form, which can be tried out here, and adds it to the open
//! workflow.

use super::{Intent, Intents, editor, form, states};
use crate::{
    document::{expression, literal},
    theme,
    widgets::{self, Tone},
    workbench::{CATALOG_UNAVAILABLE, Catalog, Page, Remote, SchemaState, Workbench},
};
use eframe::egui::{self, RichText};
use nebula_api_contract::v1::catalog::ActionSummary;

/// Panels at least this wide show the detail beside the list.
const SIDE_BY_SIDE_MIN: f32 = 860.0;

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    states::page_header(
        ui,
        "Node catalog",
        "Actions this server's release can run, with the inputs each one takes.",
        |_| {},
    );
    let actions = match &workbench.catalog {
        Catalog::NotRequested => {
            intents.push(Intent::LoadCatalog);
            states::loading(ui, "Reading the action catalog…");
            return;
        },
        Catalog::Unavailable => {
            if states::failed(ui, "The catalog is not available.", CATALOG_UNAVAILABLE) {
                intents.push(Intent::LoadCatalog);
            }
            return;
        },
        Catalog::Ready(actions) => actions.clone(),
    };
    if actions.is_empty() {
        states::empty(
            ui,
            "No actions",
            "This server's release registers no actions.",
            None,
        );
        return;
    }
    let search = ui.add(
        widgets::field(&mut workbench.catalog_page.filter)
            .hint_text("Search by name or key")
            .desired_width(f32::INFINITY),
    );
    widgets::named(ui, &search, "Search actions by name or key");
    ui.add_space(theme::SPACE_MD);
    if ui.available_width() >= SIDE_BY_SIDE_MIN {
        ui.columns(2, |columns| {
            list(&mut columns[0], workbench, &actions);
            detail(&mut columns[1], workbench, intents);
        });
    } else {
        list(ui, workbench, &actions);
        ui.add_space(theme::SPACE_LG);
        detail(ui, workbench, intents);
    }
}

/// The plugin of a catalog key: `core` for `core.sort`.
fn plugin(key: &str) -> &str {
    key.split_once('.').map_or(key, |(plugin, _)| plugin)
}

/// Where the engine runs the action, from its isolation level.
fn isolation(level: &str) -> String {
    match level {
        "None" => "Runs in the engine process.".to_owned(),
        "CapabilityGated" => {
            "Runs in the engine process, limited to the capabilities it declares.".to_owned()
        },
        other => format!("Isolation: {other}."),
    }
}

fn list(ui: &mut egui::Ui, workbench: &mut Workbench, actions: &[ActionSummary]) {
    let filter = workbench.catalog_page.filter.trim().to_lowercase();
    let matching: Vec<&ActionSummary> = actions
        .iter()
        .filter(|action| {
            filter.is_empty()
                || action.name.to_lowercase().contains(&filter)
                || action.key.to_lowercase().contains(&filter)
        })
        .collect();
    if matching.is_empty() {
        states::empty(
            ui,
            "No match",
            "No action has that in its name or key.",
            None,
        );
        return;
    }
    let mut plugins: Vec<&str> = matching.iter().map(|action| plugin(&action.key)).collect();
    plugins.sort_unstable();
    plugins.dedup();
    for name in plugins {
        ui.label(
            RichText::new(name.to_uppercase())
                .size(theme::SIZE_OVERLINE)
                .strong()
                .color(theme::TEXT_MUTED),
        );
        ui.add_space(theme::SPACE_XS);
        for action in matching.iter().filter(|action| plugin(&action.key) == name) {
            let selected = workbench.catalog_page.selected.as_deref() == Some(action.key.as_str());
            let row = egui::Button::selectable(selected, RichText::new(&action.name).strong())
                .right_text(
                    RichText::new(&action.key)
                        .monospace()
                        .color(theme::TEXT_MUTED),
                )
                .min_size(egui::vec2(ui.available_width(), 34.0));
            if ui
                .add(row)
                .on_hover_text(format!("Version {}", action.version))
                .clicked()
            {
                workbench.catalog_page.selected = Some(action.key.clone());
            }
        }
        ui.add_space(theme::SPACE_MD);
    }
}

fn detail(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    let Some(key) = workbench.catalog_page.selected.clone() else {
        theme::card_block(ui, |ui| {
            states::empty(
                ui,
                "Choose an action",
                "Its description and the form its node shows appear here.",
                None,
            );
        });
        return;
    };
    let description = workbench
        .catalog_page
        .details
        .get(&key)
        .cloned()
        .unwrap_or_default();
    if description.wants_read() {
        intents.push(Intent::LoadActionDetail(key.clone()));
    }
    let schema = workbench.schemas.get(&key).cloned();
    if schema.is_none() {
        intents.push(Intent::LoadSchema(key.clone()));
    }
    theme::card_block(ui, |ui| {
        match states::show(ui, &description, "action") {
            states::Shown::Ready(detail) => {
                ui.horizontal_wrapped(|ui| {
                    widgets::mark(
                        ui,
                        &widgets::initial(&detail.name),
                        theme::node_accent(&key),
                        32.0,
                    );
                    widgets::title(ui, &detail.name);
                    widgets::badge(ui, format!("v{}", detail.version), Tone::Neutral);
                });
                ui.label(RichText::new(&key).monospace().color(theme::TEXT_MUTED));
                ui.add_space(theme::SPACE_SM);
                ui.label(&detail.description);
                widgets::caption(ui, isolation(&detail.isolation_level));
            },
            states::Shown::Retry => {
                workbench.catalog_page.details.remove(&key);
                return;
            },
            states::Shown::Waiting => {},
        }
        ui.add_space(theme::SPACE_MD);
        add_to_workflow(ui, workbench, &key);
        ui.add_space(theme::SPACE_MD);
        widgets::section(ui, "Parameters");
        match schema {
            None | Some(SchemaState::Loading) => {
                states::loading(ui, "Reading the action's inputs…");
            },
            Some(SchemaState::Unavailable(reason)) => widgets::banner(ui, Tone::Neutral, &reason),
            Some(SchemaState::Failed(reason)) => {
                if states::failed(ui, "The action's inputs could not be read.", &reason) {
                    workbench.retry_schema(&key);
                }
            },
            Some(SchemaState::Ready(schema)) if schema.any_value => widgets::caption(
                ui,
                "This action takes free-form input that no form describes; its node edits it as JSON.",
            ),
            Some(SchemaState::Ready(schema)) => {
                widgets::caption(
                    ui,
                    "Try the form: values typed here stay on this page and change no workflow.",
                );
                ui.add_space(theme::SPACE_SM);
                let entries = workbench
                    .catalog_page
                    .preview
                    .entry(key.clone())
                    .or_default();
                let edits = form::show(
                    ui,
                    &schema,
                    entries,
                    egui::Id::new(("catalog-preview", &key)),
                );
                for edit in edits {
                    match edit {
                        form::FormEdit::Literal(field, value) => {
                            entries.insert(field, literal(value));
                        },
                        form::FormEdit::Expression(field, text) => {
                            entries.insert(field, expression(&text));
                        },
                        form::FormEdit::Clear(field) => {
                            entries.remove(&field);
                        },
                    }
                }
                // The preview's typing session is not an edit of any draft, so it is let go.
                form::take_typing_session(ui.ctx());
            },
        }
    });
}

/// Adds the action to the workflow open in the editor and goes there; without one, says how.
fn add_to_workflow(ui: &mut egui::Ui, workbench: &mut Workbench, key: &str) {
    if let Some(kind) = workbench.non_graph.get(key) {
        // The workflow compiler admits only stateless, stateful, control and agent actions as
        // nodes; a trigger is bound on the Triggers page instead.
        widgets::caption(
            ui,
            format!(
                "A {kind} action is not a node of a workflow graph, so it cannot be added to one."
            ),
        );
        return;
    }
    let open = workbench
        .session
        .draft()
        .map(|draft| draft.base.workflow.name.clone());
    match open {
        Some(name) => {
            if ui
                .add(widgets::primary_button(&format!(
                    "Add to \u{201c}{name}\u{201d}"
                )))
                .on_hover_text("Add a node for this action to the open workflow")
                .clicked()
            {
                let action = workbench
                    .catalog_page
                    .details
                    .get(key)
                    .and_then(Remote::value)
                    .map_or_else(|| key.to_owned(), |detail| detail.name.clone());
                editor::add_node(workbench, key, &action);
                workbench.go(Page::Editor);
            }
        },
        None => widgets::caption(
            ui,
            "Open a workflow in the editor to add this action to it.",
        ),
    }
}
