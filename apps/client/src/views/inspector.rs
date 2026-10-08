//! The node sidebar, which slides in from the right when a node is selected. Parameters are a form built
//! from the action's schema; Settings hold the name, connections, removal and the raw parameter JSON;
//! Output shows what the node produced in the chosen run. Every change here is a local draft command.
use super::{Intent, Intents, form};
use crate::{
    document::Draft,
    schema::display,
    theme,
    widgets::{self, Tone},
    workbench::{InspectorTab, SchemaState, Workbench},
};
use eframe::egui::{self, RichText};
use nebula_api_contract::v1::execution::ExecutionNodeOutput;
use serde_json::{Map, Value};

/// A connection shown in the inspector, with its endpoints and the label it is drawn under.
struct Link {
    from: String,
    to: String,
    label: String,
}

/// What the sidebar needs from the node, copied out so the frame can change the workbench.
struct NodeView {
    id: String,
    name: String,
    action: String,
    parameters: Map<String, Value>,
    links: Vec<Link>,
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
            parameters: node["parameters"].as_object().cloned().unwrap_or_default(),
            links: draft
                .links(id)
                .into_iter()
                .map(|(from, to)| Link {
                    label: format!("{} → {}", draft.node_name(&from), draft.node_name(&to)),
                    from,
                    to,
                })
                .collect(),
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
    if header(ui, &node) {
        workbench.selected_node = None;
        workbench.parameter.close();
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
                    InspectorTab::Parameters => parameters(ui, workbench, &node, intents),
                    InspectorTab::Settings => settings(ui, workbench, &node),
                    InspectorTab::Output => output(ui, workbench, &node),
                });
        });
}

/// The node's badge, name and action, with Close. Returns true when Close was clicked.
fn header(ui: &mut egui::Ui, node: &NodeView) -> bool {
    ui.horizontal(|ui| {
        let letter: String = node.name.chars().take(1).collect::<String>().to_uppercase();
        widgets::mark(ui, &letter, theme::node_accent(&node.action), 32.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(RichText::new(&node.name).size(16.0).strong());
            widgets::caption(ui, node.action.as_str());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::Button::new(RichText::new("×").size(20.0).color(theme::TEXT_MUTED))
                    .frame_when_inactive(false)
                    .min_size(egui::vec2(28.0, 28.0)),
            )
            .on_hover_text("Close (Esc)")
            .clicked()
        })
        .inner
    })
    .inner
}

fn parameters(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    node: &NodeView,
    intents: &mut Intents,
) {
    let state = workbench.schemas.get(&node.action).cloned();
    match state {
        None => {
            // Asked once per action; the app drops the request while another one runs and asks again.
            intents.push(Intent::LoadSchema(node.action.clone()));
            loading(ui);
        },
        Some(SchemaState::Loading) => loading(ui),
        Some(SchemaState::Ready(schema)) => {
            if schema.fields.is_empty() {
                widgets::caption(ui, "This action takes no parameters.");
            }
            let edits = form::show(
                ui,
                &schema,
                &node.parameters,
                egui::Id::new(("node-form", &node.id)),
            );
            apply(workbench, &node.id, edits);
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
            raw_parameters(ui, workbench, node);
        },
    }
}

fn loading(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spinner();
        widgets::caption(ui, "Reading the action's inputs…");
    });
}

/// Applies form edits as draft commands; each is one undo step.
fn apply(workbench: &mut Workbench, node: &str, edits: Vec<form::FormEdit>) {
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    for edit in edits {
        let result = match edit {
            form::FormEdit::Literal(key, value) => draft.set_literal(node, &key, value),
            form::FormEdit::Expression(key, expression) => {
                draft.set_expression(node, &key, &expression)
            },
            form::FormEdit::Clear(key) => draft.clear_parameter(node, &key),
        };
        // A successful edit shows in the form itself; only a refusal needs words.
        if let Err(error) = result {
            workbench.feedback.error(error.to_string());
            return;
        }
    }
}

fn settings(ui: &mut egui::Ui, workbench: &mut Workbench, node: &NodeView) {
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
    for link in &node.links {
        ui.horizontal_wrapped(|ui| {
            widgets::caption(ui, link.label.as_str());
            if ui.small_button("Disconnect").clicked() {
                disconnect(workbench, &link.from, &link.to);
            }
        });
    }

    ui.add_space(theme::SPACE_MD);
    widgets::section(ui, "Parameters as JSON");
    raw_parameters(ui, workbench, node);

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
    let Some(status) = &workbench.status else {
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
    widgets::badge(ui, format!("{:?}", result.status), Tone::Neutral);
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

fn disconnect(workbench: &mut Workbench, from: &str, to: &str) {
    let Some(draft) = workbench.session.draft_mut() else {
        return;
    };
    let result = draft.disconnect(from, to);
    workbench
        .feedback
        .report(result, &format!("Disconnected {from} from {to}."));
}

/// Every parameter entry as JSON, for nodes without a schema and for entries the schema does not
/// declare. A literal opens in the JSON editor; other kinds are shown as they are stored.
fn raw_parameters(ui: &mut egui::Ui, workbench: &mut Workbench, node: &NodeView) {
    if node.parameters.is_empty() {
        widgets::caption(ui, "No parameters are set.");
    }
    for (name, entry) in &node.parameters {
        ui.horizontal_wrapped(|ui| {
            let selected =
                workbench.parameter.node == node.id && workbench.parameter.parameter == *name;
            let literal = entry["type"] == "literal";
            let row = ui.add_enabled(
                literal,
                egui::Button::new(name.as_str())
                    .selected(selected)
                    .min_size(egui::vec2(120.0, 28.0)),
            );
            if row.clicked() {
                let text = serde_json::to_string_pretty(&entry["value"]).unwrap_or_default();
                workbench.parameter.open(&node.id, name, text);
            }
            let preview: String = if literal {
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
    widgets::caption(ui, format!("{} as JSON", workbench.parameter.parameter));
    ui.add(
        egui::TextEdit::multiline(&mut workbench.parameter.text)
            .code_editor()
            .desired_rows(6)
            .desired_width(f32::INFINITY),
    );
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
    let result = draft.edit(
        &workbench.parameter.node,
        &workbench.parameter.parameter,
        &workbench.parameter.text,
    );
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
        ui.add(
            egui::TextEdit::singleline(&mut name)
                .hint_text("New parameter name")
                .desired_width(ui.available_width() - 64.0),
        );
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
