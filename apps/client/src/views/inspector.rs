//! The node sidebar, which slides in from the right when a node is selected. Parameters are a form built
//! from the action's schema; Settings hold the name, connections, removal and the raw parameter JSON;
//! Output shows what the node produced in the chosen run. Every change here is a local draft command.
use super::{Intent, Intents, form, status};
use crate::{
    document::{Draft, Link, catalog_key, expression, literal, source_ports},
    schema::display,
    theme,
    widgets::{self, Tone},
    workbench::{InspectorTab, SchemaState, Workbench},
};
use eframe::egui::{self, RichText};
use nebula_api_contract::v1::execution::ExecutionNodeOutput;
use serde_json::{Map, Value};

/// A connection shown in the inspector, with the label it is drawn under.
struct LinkRow {
    link: Link,
    label: String,
}

impl LinkRow {
    /// Names the nodes, and the ports when they are not the defaults, so parallel routes differ.
    fn of(draft: &Draft, link: Link) -> Self {
        let source = match link.source_port() {
            "out" => draft.node_name(&link.from),
            port => format!("{} ({port})", draft.node_name(&link.from)),
        };
        let target = match &link.to_port {
            Some(port) => format!("{} ({port})", draft.node_name(&link.to)),
            None => draft.node_name(&link.to),
        };
        Self {
            label: format!("{source} → {target}"),
            link,
        }
    }
}

/// What the sidebar needs from the node, copied out so the frame can change the workbench.
struct NodeView {
    id: String,
    name: String,
    action: String,
    /// `plugin.action`, the key the catalog knows the action by.
    catalog: String,
    parameters: Map<String, Value>,
    links: Vec<LinkRow>,
    /// Output ports a link from this node can leave by.
    ports: Vec<String>,
}

impl NodeView {
    fn of(draft: &Draft, id: &str) -> Option<Self> {
        let node = draft.definition["nodes"]
            .as_array()?
            .iter()
            .find(|node| node["id"].as_str() == Some(id))?;
        Some(Self {
            id: id.to_owned(),
            name: node["name"].as_str().unwrap_or(id).to_owned(),
            action: node["action_key"].as_str().unwrap_or_default().to_owned(),
            catalog: catalog_key(node),
            parameters: node["parameters"].as_object().cloned().unwrap_or_default(),
            links: draft
                .links(id)
                .into_iter()
                .map(|link| LinkRow::of(draft, link))
                .collect(),
            ports: source_ports(node),
        })
    }
}

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench, intents: &mut Intents) {
    let Some(node_id) = workbench.selected_node.clone() else {
        return;
    };
    let Some(node) = workbench
        .session
        .draft()
        .and_then(|draft| NodeView::of(draft, &node_id))
    else {
        // A replay or a server read removed the node, so there is nothing left to inspect.
        workbench.selected_node = None;
        return;
    };
    // Everything the panel keeps between frames belongs to this node of this workflow in this
    // workspace, so a revealed secret never shows through on another node or workflow.
    let Some(key) = workbench.session.selected.clone() else {
        return;
    };
    let scope = egui::Id::new((
        "node-form",
        &key.context.endpoint,
        &key.context.principal,
        &key.context.organization,
        &key.context.workspace_selector,
        &key.workflow,
        &node.id,
    ));
    let shown_id = egui::Id::new("inspector-shown");
    if ui.data(|data| data.get_temp::<egui::Id>(shown_id)) != Some(scope) {
        form::hide_secrets(ui.ctx());
        ui.data_mut(|data| data.insert_temp(shown_id, scope));
    }
    if header(ui, &node) {
        workbench.selected_node = None;
        workbench.parameter.close();
        form::hide_secrets(ui.ctx());
        ui.data_mut(|data| data.remove::<egui::Id>(shown_id));
        return;
    }
    ui.add_space(theme::SPACE_SM);
    widgets::tabs(
        ui,
        &mut workbench.inspector_tab,
        &[
            (InspectorTab::Parameters, "Parameters"),
            (InspectorTab::Settings, "Settings"),
            (InspectorTab::Output, "Output"),
        ],
    );
    // Only the tab body scrolls; the node and its tabs stay at the top of the panel.
    let tab = workbench.inspector_tab;
    egui::ScrollArea::vertical()
        .id_salt(("inspector", tab as u8))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Room on the right for the floating scroll bar, so it never covers an input's edge.
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    right: theme::SPACE_MD as i8,
                    ..egui::Margin::ZERO
                })
                .show(ui, |ui| match tab {
                    InspectorTab::Parameters => parameters(ui, workbench, &node, scope, intents),
                    InspectorTab::Settings => settings(ui, workbench, &node, scope),
                    InspectorTab::Output => output(ui, workbench, &node),
                });
        });
}

/// The node's badge, name and action, with Close. Returns true when Close was clicked.
fn header(ui: &mut egui::Ui, node: &NodeView) -> bool {
    ui.horizontal(|ui| {
        widgets::mark(
            ui,
            &widgets::initial(&node.name),
            theme::node_accent(&node.action),
            32.0,
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(
                RichText::new(&node.name)
                    .size(theme::SIZE_PANEL_TITLE)
                    .strong(),
            );
            widgets::caption(ui, node.action.as_str());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = ui
                .add(
                    egui::Button::new(RichText::new("×").size(20.0).color(theme::TEXT_MUTED))
                        .frame_when_inactive(false)
                        .min_size(egui::vec2(28.0, 28.0)),
                )
                .on_hover_text("Close (Esc)");
            widgets::named(ui, &close, "Close the node panel");
            close.clicked()
        })
        .inner
    })
    .inner
}

fn parameters(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    node: &NodeView,
    scope: egui::Id,
    intents: &mut Intents,
) {
    if let Some(kind) = workbench.non_graph.get(&node.catalog) {
        widgets::banner(
            ui,
            Tone::Warning,
            &format!(
                "{} is a {kind} action, which a workflow graph cannot run as a node; publishing \
                 rejects it. Remove the node, or bind the action as a trigger.",
                node.action
            ),
        );
        ui.add_space(theme::SPACE_SM);
    }
    let state = workbench.schemas.get(&node.catalog).cloned();
    match state {
        None => {
            // Asked once per action; the app drops the request while another one runs and asks again.
            intents.push(Intent::LoadSchema(node.catalog.clone()));
            loading(ui);
        },
        Some(SchemaState::Loading) => loading(ui),
        Some(SchemaState::Ready(schema)) if schema.any_value => {
            widgets::caption(
                ui,
                "This action takes free-form input that no form describes. Edit it as JSON.",
            );
            raw_parameters(ui, workbench, node, scope);
        },
        Some(SchemaState::Ready(schema)) => {
            if schema.fields.is_empty() {
                widgets::caption(ui, "This action takes no parameters.");
            }
            let edits = form::show(ui, &schema, &node.parameters, scope);
            let typing = form::take_typing_session(ui.ctx());
            apply(workbench, &node.id, edits, typing);
            let unknown: Vec<&str> = node
                .parameters
                .keys()
                .filter(|key| !schema.fields.iter().any(|field| &field.key == *key))
                .map(String::as_str)
                .collect();
            if !unknown.is_empty() {
                widgets::caption(
                    ui,
                    format!(
                        "Also set, but not declared by the action: {}. Edit them in Settings.",
                        unknown.join(", ")
                    ),
                );
            }
        },
        Some(SchemaState::Unavailable(reason)) => {
            widgets::banner(ui, Tone::Neutral, &reason);
            ui.add_space(theme::SPACE_SM);
            widgets::caption(ui, "Edit the parameters as JSON instead.");
            raw_parameters(ui, workbench, node, scope);
        },
        Some(SchemaState::Failed(reason)) => {
            widgets::banner(
                ui,
                Tone::Warning,
                &format!("The action's inputs could not be read. {reason}"),
            );
            ui.add_space(theme::SPACE_SM);
            if ui.button("Try again").clicked() {
                workbench.retry_schema(&node.catalog);
            }
            ui.add_space(theme::SPACE_SM);
            widgets::caption(ui, "Meanwhile, edit the parameters as JSON.");
            raw_parameters(ui, workbench, node, scope);
        },
    }
}

fn loading(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spinner();
        widgets::caption(ui, "Reading the action's inputs…");
    });
}

/// Applies form edits as draft commands. A click is one undo step; text typed in one focus is one
/// step however many keystrokes it took.
fn apply(workbench: &mut Workbench, node: &str, edits: Vec<form::FormEdit>, typing: Option<u64>) {
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    for edit in edits {
        let (key, entry) = match edit {
            form::FormEdit::Literal(key, value) => (key, Some(literal(value))),
            form::FormEdit::Expression(key, text) => (key, Some(expression(&text))),
            form::FormEdit::Clear(key) => (key, None),
        };
        let result = match typing {
            Some(session) => draft.type_parameter(node, &key, entry, session),
            None => draft.set_entry(node, &key, entry),
        };
        // A successful edit shows in the form itself; only a refusal needs words.
        if let Err(error) = result {
            workbench.feedback.error(error.to_string());
            return;
        }
    }
}

fn settings(ui: &mut egui::Ui, workbench: &mut Workbench, node: &NodeView, scope: egui::Id) {
    widgets::labeled_field(ui, "Name", &mut workbench.rename, false);
    let renamed = workbench.rename.trim().to_owned();
    let changed = !renamed.is_empty() && renamed != node.name;
    if ui
        .add_enabled(changed, egui::Button::new("Rename"))
        .clicked()
    {
        rename(workbench, &node.id, &renamed);
    }
    widgets::caption(ui, format!("Node id {}", node.id));

    ui.add_space(theme::SPACE_MD);
    widgets::section(ui, "Connections");
    if node.links.is_empty() {
        widgets::caption(
            ui,
            "Not connected. Drag from an output port to another node's input port.",
        );
    }
    for row in &node.links {
        ui.horizontal_wrapped(|ui| {
            widgets::caption(ui, row.label.as_str());
            // A link leaving this node chooses its output port, so branches of `if` and `switch`
            // can be drawn from the canvas and routed here.
            if row.link.from == node.id {
                let current = row.link.source_port().to_owned();
                let mut chosen = current.clone();
                let choice = egui::ComboBox::from_id_salt(("link-port", &row.label))
                    .selected_text(format!("from {current}"))
                    .show_ui(ui, |ui| {
                        for port in &node.ports {
                            ui.selectable_value(&mut chosen, port.clone(), port);
                        }
                    })
                    .response
                    .on_hover_text("The output port this connection leaves by");
                widgets::named(ui, &choice, &format!("Output port of {}", row.label));
                if chosen != current {
                    reroute(workbench, row, chosen);
                }
            }
            if ui.small_button("Disconnect").clicked() {
                disconnect(workbench, row);
            }
        });
    }

    ui.add_space(theme::SPACE_MD);
    widgets::section(ui, "Parameters as JSON");
    raw_parameters(ui, workbench, node, scope);

    ui.add_space(theme::SPACE_LG);
    if ui
        .add(widgets::danger_button("Remove node"))
        .on_hover_text("Remove the node and its connections (Delete)")
        .clicked()
    {
        remove(workbench, &node.id, &node.name);
    }
}

/// What the node produced in the chosen run, from the runs panel.
fn output(ui: &mut egui::Ui, workbench: &Workbench, node: &NodeView) {
    let chosen = workbench
        .session
        .draft()
        .and_then(|draft| draft.execution_id.as_deref());
    let shown = workbench
        .status
        .as_ref()
        .filter(|status| Some(status.execution.id.as_str()) == chosen);
    let Some(status) = shown else {
        widgets::caption(
            ui,
            "Choose a run in the runs panel to see what this node produced.",
        );
        return;
    };
    let Some(result) = status.nodes.get(&node.id) else {
        widgets::caption(ui, "The chosen run did not reach this node.");
        return;
    };
    widgets::badge(
        ui,
        status::node_label(result.status),
        status::node_tone(result.status),
    );
    ui.add_space(theme::SPACE_SM);
    match &result.output {
        Some(ExecutionNodeOutput::Inline { value }) => {
            let pretty = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
            egui::ScrollArea::vertical()
                .id_salt("node-output")
                .max_height(360.0)
                .show(ui, |ui| {
                    ui.label(RichText::new(pretty).monospace());
                });
        },
        Some(_) => widgets::caption(
            ui,
            "The output is stored outside the run record and is not shown here.",
        ),
        None => widgets::caption(ui, "No output was recorded."),
    }
    if let Some(error) = &result.error {
        let reason = error.message.as_deref().unwrap_or(&error.category);
        ui.label(RichText::new(format!("{}: {reason}", error.code)).color(theme::DANGER));
    }
}

fn rename(workbench: &mut Workbench, node: &str, name: &str) {
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    let result = draft.rename_node(node, name);
    workbench
        .feedback
        .report(result, "Node renamed in your draft.");
}

/// Removes the selected node, as the Delete key does. Undo brings it back.
pub(crate) fn remove_selected(workbench: &mut Workbench) {
    let (Some(node), Some(draft)) = (workbench.selected_node.clone(), workbench.session.draft())
    else {
        return;
    };
    let name = draft.node_name(&node);
    remove(workbench, &node, &name);
}

fn remove(workbench: &mut Workbench, node: &str, name: &str) {
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    let result = draft.remove_node(node);
    if result.is_ok() {
        workbench.selected_node = None;
        workbench.parameter.close();
    }
    workbench
        .feedback
        .report(result, &format!("Removed {name} and its connections."));
}

fn reroute(workbench: &mut Workbench, row: &LinkRow, port: String) {
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    let result = draft.reroute(&row.link, Some(port.clone()));
    workbench
        .feedback
        .report(result, &format!("{} now leaves by {port}.", row.label));
}

fn disconnect(workbench: &mut Workbench, row: &LinkRow) {
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    let result = draft.disconnect(&row.link);
    workbench
        .feedback
        .report(result, &format!("Removed the connection {}.", row.label));
}

/// Every parameter entry as JSON, for nodes without a schema and for entries the schema does not
/// declare. A literal opens its value in the JSON editor; any other kind opens the whole stored
/// entry. A parameter the schema declares secret stays masked until it is shown on purpose.
fn raw_parameters(ui: &mut egui::Ui, workbench: &mut Workbench, node: &NodeView, scope: egui::Id) {
    if node.parameters.is_empty() {
        widgets::caption(ui, "No parameters are set.");
    }
    let secrets: Vec<String> = match workbench.schemas.get(&node.catalog) {
        Some(SchemaState::Ready(schema)) => schema
            .fields
            .iter()
            .filter(|field| field.holds_secret())
            .map(|field| field.key.clone())
            .collect(),
        _ => Vec::new(),
    };
    for (name, entry) in &node.parameters {
        ui.horizontal_wrapped(|ui| {
            let selected =
                workbench.parameter.node == node.id && workbench.parameter.parameter == *name;
            let literal = entry["type"] == "literal";
            let secret = secrets.contains(name);
            let shown = !secret || form::reveal_toggle(ui, scope.with(("raw", name)));
            let row = ui.add_enabled(
                shown,
                egui::Button::new(name.as_str())
                    .selected(selected)
                    .min_size(egui::vec2(120.0, 28.0)),
            );
            if row.clicked() {
                if literal {
                    let text = serde_json::to_string_pretty(&entry["value"]).unwrap_or_default();
                    workbench.parameter.open(&node.id, name, text);
                } else {
                    let text = serde_json::to_string_pretty(entry).unwrap_or_default();
                    workbench.parameter.open_entry(&node.id, name, text);
                }
            }
            let preview: String = if !shown {
                "•••••• secret".to_owned()
            } else if literal {
                display(&entry["value"])
            } else {
                format!(
                    "{} · {}",
                    entry["type"].as_str().unwrap_or("unknown"),
                    entry
                )
            }
            .chars()
            .take(64)
            .collect();
            widgets::caption(ui, preview);
        });
    }
    parameter_editor(ui, workbench);
    add_parameter(ui, workbench, &node.id);
}

/// The open literal. Applying it changes the draft locally; saving is a separate step.
fn parameter_editor(ui: &mut egui::Ui, workbench: &mut Workbench) {
    let owns_selection =
        workbench.selected_node.as_deref() == Some(workbench.parameter.node.as_str());
    if !workbench.parameter.is_open() || !owns_selection {
        return;
    }
    ui.add_space(theme::SPACE_SM);
    let what = if workbench.parameter.entry {
        "stored entry"
    } else {
        "value"
    };
    let caption = format!("{} {what} as JSON", workbench.parameter.parameter);
    widgets::caption(ui, &caption);
    let editor = ui.add(
        egui::TextEdit::multiline(&mut workbench.parameter.text)
            .code_editor()
            .desired_rows(6)
            .desired_width(f32::INFINITY),
    );
    widgets::named(ui, &editor, &caption);
    ui.horizontal(|ui| {
        if ui.add(widgets::primary_button("Apply")).clicked() {
            apply_parameter(workbench);
        }
        if ui.button("Close").clicked() {
            workbench.parameter.close();
        }
    });
}

fn apply_parameter(workbench: &mut Workbench) {
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    let selection = &workbench.parameter;
    let result = if selection.entry {
        draft.edit_entry(&selection.node, &selection.parameter, &selection.text)
    } else {
        draft.edit(&selection.node, &selection.parameter, &selection.text)
    };
    workbench
        .feedback
        .report(result, "Parameter edited in your draft.");
}

/// A new literal parameter by name, for actions without a schema.
fn add_parameter(ui: &mut egui::Ui, workbench: &mut Workbench, node: &str) {
    let name_id = egui::Id::new(("new-parameter", node));
    let mut name = ui
        .data(|data| data.get_temp::<String>(name_id))
        .unwrap_or_default();
    ui.add_space(theme::SPACE_SM);
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut name)
                .hint_text("New parameter name")
                .desired_width(ui.available_width() - 64.0),
        );
        widgets::named(ui, &field, "New parameter name");
        let ready = !name.trim().is_empty();
        if ui.add_enabled(ready, egui::Button::new("Add")).clicked() {
            let key = name.trim().to_owned();
            if let Some(draft) = workbench.session.draft_mut() {
                let result = draft.set_literal(node, &key, Value::String(String::new()));
                if result.is_ok() {
                    workbench.parameter.open(node, &key, "\"\"".to_owned());
                }
                workbench
                    .feedback
                    .report(result, "Parameter added to your draft.");
            }
            name.clear();
        }
    });
    ui.data_mut(|data| data.insert_temp(name_id, name));
}
