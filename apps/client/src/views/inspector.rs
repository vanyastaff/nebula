//! Inspector for the selected node: its name, connections, parameters and removal. Every change here is
//! a local draft command. Only the editor's toolbar starts network work.
use crate::{document::Draft, theme, widgets, workbench::Workbench};
use eframe::egui;
use serde_json::{Map, Value};

/// A connection shown in the inspector, with its endpoints and the label it is drawn under.
struct Link {
    from: String,
    to: String,
    label: String,
}

pub(crate) fn show(ui: &mut egui::Ui, workbench: &mut Workbench) {
    let Some(node_id) = workbench.selected_node.clone() else {
        widgets::caption(
            ui,
            "Select a node on the graph to rename it, edit its parameters or change its connections.",
        );
        return;
    };
    let Some(draft) = workbench.session.draft() else {
        return;
    };
    let Some(node) = find_node(draft, &node_id) else {
        // A replay or a server read removed the node, so there is nothing left to inspect.
        workbench.selected_node = None;
        return;
    };
    let name = node["name"].as_str().unwrap_or(&node_id).to_owned();
    let action = node["action_key"].as_str().unwrap_or_default().to_owned();
    let parameters: Map<String, Value> =
        node["parameters"].as_object().cloned().unwrap_or_default();
    let links: Vec<Link> = draft
        .links(&node_id)
        .into_iter()
        .map(|(from, to)| Link {
            label: format!("{} → {}", draft.node_name(&from), draft.node_name(&to)),
            from,
            to,
        })
        .collect();

    theme::card_block(ui, |ui| {
        widgets::section(ui, &name);
        widgets::caption(ui, format!("{action} · id {node_id}"));
        ui.add_space(theme::SPACE_SM);
        widgets::labeled_field(ui, "Name", &mut workbench.rename, false);
        ui.horizontal_wrapped(|ui| {
            let renamed = workbench.rename.trim().to_owned();
            let changed = !renamed.is_empty() && renamed != name;
            if ui
                .add_enabled(changed, egui::Button::new("Rename"))
                .clicked()
            {
                rename(workbench, &node_id, &renamed);
            }
            if ui.add(widgets::danger_button("Remove node")).clicked() {
                remove(workbench, &node_id, &name);
            }
        });

        ui.add_space(theme::SPACE_SM);
        widgets::section(ui, "Connections");
        if links.is_empty() {
            widgets::caption(
                ui,
                "Not connected. Drag from an output port to another node's input port.",
            );
        }
        for link in &links {
            ui.horizontal_wrapped(|ui| {
                widgets::caption(ui, &link.label);
                if ui.button("Disconnect").clicked() {
                    disconnect(workbench, &link.from, &link.to);
                }
            });
        }

        ui.add_space(theme::SPACE_SM);
        widgets::section(ui, "Parameters");
        if parameters.is_empty() {
            widgets::caption(ui, "This node has no configurable parameters.");
        }
        for (parameter, value) in &parameters {
            parameter_row(ui, workbench, &node_id, parameter, value);
        }
        parameter_editor(ui, workbench);
    });
}

fn find_node(draft: &Draft, id: &str) -> Option<Value> {
    draft.definition["nodes"]
        .as_array()?
        .iter()
        .find(|node| node["id"].as_str() == Some(id))
        .cloned()
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

fn parameter_row(
    ui: &mut egui::Ui,
    workbench: &mut Workbench,
    node_id: &str,
    name: &str,
    value: &Value,
) {
    ui.horizontal_wrapped(|ui| {
        if value["type"].as_str() == Some("literal") {
            let selected =
                workbench.parameter.node == node_id && workbench.parameter.parameter == name;
            let entry = egui::Button::new(name)
                .selected(selected)
                .min_size(egui::vec2(140.0, 32.0));
            if ui.add(entry).clicked() {
                let text = serde_json::to_string_pretty(&value["value"]).unwrap_or_default();
                workbench.parameter.open(node_id, name, text);
            }
            let preview: String = value["value"].to_string().chars().take(64).collect();
            widgets::caption(ui, preview);
        } else {
            ui.add_enabled(
                false,
                egui::Button::new(name).min_size(egui::vec2(140.0, 32.0)),
            );
            widgets::caption(
                ui,
                format!(
                    "{} parameter · read-only in this release",
                    value["type"].as_str().unwrap_or("unknown")
                ),
            );
        }
    });
}

/// The open parameter. Applying it changes the draft locally; saving is a separate step.
fn parameter_editor(ui: &mut egui::Ui, workbench: &mut Workbench) {
    let owns_selection =
        workbench.selected_node.as_deref() == Some(workbench.parameter.node.as_str());
    if !workbench.parameter.is_open() || !owns_selection {
        return;
    }
    ui.add_space(theme::SPACE_MD);
    widgets::section(ui, &workbench.parameter.parameter);
    widgets::caption(ui, "Edit the JSON value, then apply it to your draft.");
    ui.add_space(theme::SPACE_XS);
    ui.add(
        egui::TextEdit::multiline(&mut workbench.parameter.text)
            .code_editor()
            .desired_rows(8)
            .desired_width(f32::INFINITY),
    );
    ui.horizontal(|ui| {
        if ui
            .add(widgets::primary_button("Apply parameter edit"))
            .clicked()
        {
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
