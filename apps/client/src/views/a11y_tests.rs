//! What assistive technology reads: each page is drawn in a headless frame with AccessKit on, on the
//! demo workspace's data, and the tree it hands to the platform is checked for names and roles.

use super::{Intents, catalog, credentials, executions, nav, settings, team, triggers, workflows};
use crate::{
    api::{Backend, SignedIn},
    demo::{self, Demo},
    effects::{Reply, RequestKind},
    transport::ExecutionQuery,
    workbench::{Remote, Workbench},
};
use eframe::egui::{self, accesskit::Role};

/// A workbench signed in to a fresh demo, with every page's data read.
fn demo_workspace() -> Workbench {
    demo_world().0
}

/// The same, with the demo's world for reads a test makes itself.
fn demo_world() -> (Workbench, Demo) {
    let world = Demo::new().unwrap();
    let mut workbench = Workbench::new(String::new());
    let backend = Backend::Demo(world.clone());
    workbench.begin_sign_in(backend.clone());
    let stamp = workbench.session.begin().unwrap();
    let profile = world.me().unwrap();
    workbench.receive(
        stamp,
        RequestKind::Connect,
        Ok(Reply::Connected(SignedIn { backend, profile })),
    );
    let stamp = workbench.session.begin().unwrap();
    workbench.receive(
        stamp,
        RequestKind::Read,
        Ok(Reply::Listed(world.list(1).unwrap())),
    );
    let stamp = workbench.session.begin().unwrap();
    workbench.receive(
        stamp,
        RequestKind::Catalog,
        Ok(Reply::Actions(world.actions().unwrap())),
    );
    let query = ExecutionQuery {
        workflow: None,
        statuses: String::new(),
        cursor: None,
        limit: 25,
    };
    workbench.executions.list = Remote::Ready(world.executions(&query).unwrap().items);
    workbench.credentials.list = Remote::Ready(world.credentials().unwrap().credentials);
    workbench.credentials.types = Remote::Ready(world.credential_types().unwrap().types);
    let documents = world
        .list(1)
        .unwrap()
        .workflows
        .iter()
        .map(|workflow| world.load(&workflow.id).unwrap())
        .collect();
    workbench.triggers.documents = Remote::Ready(documents);
    workbench.settings.profile = Remote::Ready(world.me().unwrap());
    workbench.settings.tokens = Remote::Ready(world.tokens().unwrap().tokens);
    workbench.team.organization = Remote::Ready(world.org_members(demo::ORG).unwrap().members);
    workbench.team.workspace = Remote::Ready(world.workspace_members().unwrap().members);
    (workbench, world)
}

/// Role and name of every node in the tree one frame of `draw` produces.
fn tree(
    workbench: &mut Workbench,
    mut draw: impl FnMut(&mut egui::Ui, &mut Workbench, &mut Intents),
) -> Vec<(Role, String)> {
    let context = egui::Context::default();
    context.enable_accesskit();
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1440.0, 900.0),
        )),
        ..Default::default()
    };
    let mut intents = Intents::new();
    // The first frame measures; the second lays out with what it measured.
    let mut output = None;
    for _ in 0..2 {
        let mut frame = context.run_ui(input(), |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| draw(ui, workbench, &mut intents));
        });
        // No renderer takes the font atlas here.
        frame.textures_delta.clear();
        output = Some(frame);
    }
    output
        .unwrap()
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .into_iter()
        // egui names a control by its label and reads a text label out as its value.
        .map(|(_, node)| {
            let name = node.label().or_else(|| node.value()).unwrap_or_default();
            (node.role(), name.to_owned())
        })
        .collect()
}

fn names(nodes: &[(Role, String)], role: Role) -> Vec<&str> {
    nodes
        .iter()
        .filter(|(found, _)| *found == role)
        .map(|(_, name)| name.as_str())
        .collect()
}

/// Every button on the page has a name, so a screen reader never announces a bare "button".
fn every_button_is_named(nodes: &[(Role, String)]) {
    let buttons = names(nodes, Role::Button);
    assert!(!buttons.is_empty());
    assert!(
        buttons.iter().all(|name| !name.trim().is_empty()),
        "unnamed button among {buttons:?}"
    );
}

#[test]
fn the_rail_names_every_page() {
    let mut workbench = demo_workspace();
    let nodes = tree(&mut workbench, |ui, workbench, _| nav::rail(ui, workbench));

    let buttons = names(&nodes, Role::Button);
    for page in [
        "Workflows",
        "Executions",
        "Node catalog",
        "Triggers",
        "Credentials",
        "Team",
        "Settings",
    ] {
        assert!(buttons.contains(&page), "{page} missing from {buttons:?}");
    }
}

#[test]
fn a_workflow_card_is_one_named_button() {
    let mut workbench = demo_workspace();
    let nodes = tree(&mut workbench, workflows::show);

    every_button_is_named(&nodes);
    assert!(names(&nodes, Role::Button).contains(&"Open Order fulfillment"));
}

#[test]
fn a_run_row_says_what_it_opens_and_its_status_is_read_as_text() {
    let mut workbench = demo_workspace();
    let nodes = tree(&mut workbench, executions::show);

    every_button_is_named(&nodes);
    assert!(
        names(&nodes, Role::Button)
            .iter()
            .any(|name| name.starts_with("Show the completed run of "))
    );
    assert!(names(&nodes, Role::Label).contains(&"Completed"));
}

#[test]
fn a_run_timeline_names_each_node_and_its_state() {
    let (mut workbench, world) = demo_world();
    let id = workbench.executions.list.value().unwrap()[0].id.clone();
    let detail = world.status(&id).unwrap();
    workbench.go(crate::workbench::Page::Executions);
    workbench.executions.selected = Some(id);
    workbench.executions.detail = Remote::Ready(Box::new(detail.clone()));
    let nodes = tree(&mut workbench, executions::show);

    every_button_is_named(&nodes);
    let all: Vec<&str> = nodes.iter().map(|(_, name)| name.as_str()).collect();
    for node in detail.nodes.keys() {
        assert!(
            all.iter().any(|name| name.contains(node.as_str())),
            "timeline node {node} unnamed in {all:?}"
        );
    }
}

#[test]
fn the_other_pages_name_every_button() {
    let pages: [fn(&mut egui::Ui, &mut Workbench, &mut Intents); 5] = [
        catalog::show,
        triggers::show,
        credentials::show,
        team::show,
        settings::show,
    ];
    for page in pages {
        let mut workbench = demo_workspace();
        every_button_is_named(&tree(&mut workbench, page));
    }
}
