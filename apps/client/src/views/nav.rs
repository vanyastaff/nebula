//! Moving between the pages of a workspace: a rail on wide windows, a row of tabs on narrow ones,
//! keyboard shortcuts, and the sheet that lists every shortcut.

use crate::{
    theme, widgets,
    workbench::{Page, Workbench},
};
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, RichText};

/// The rail beside the page on wide windows. Each entry is a labelled button, so screen readers name
/// it, and its shortcut shows on hover.
pub(crate) fn rail(ui: &mut egui::Ui, workbench: &mut Workbench) {
    ui.add_space(theme::SPACE_SM);
    for (index, page) in Page::NAVIGATION.into_iter().enumerate() {
        let active = workbench.page.section() == page;
        let entry =
            egui::Button::selectable(active, RichText::new(page.title()).size(theme::SIZE_BODY))
                .min_size(egui::vec2(ui.available_width(), 34.0));
        if ui
            .add(entry)
            .on_hover_text(format!("Alt+{}", index + 1))
            .clicked()
        {
            workbench.go(page);
        }
    }
}

/// The pages as a row of tabs under the top bar, for windows too narrow for the rail. The row wraps
/// rather than scrolling sideways.
pub(crate) fn tabs(ui: &mut egui::Ui, workbench: &mut Workbench) {
    ui.horizontal_wrapped(|ui| {
        for page in Page::NAVIGATION {
            let active = workbench.page.section() == page;
            if ui.selectable_label(active, page.title()).clicked() {
                workbench.go(page);
            }
        }
    });
}

/// Shortcuts that work on every page while no text field has the keyboard: Alt with a number opens a
/// page, `?` shows the shortcut sheet, Escape closes it.
pub(crate) fn shortcuts(context: &egui::Context, workbench: &mut Workbench) {
    if workbench.shortcuts_open
        && context.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
    {
        workbench.shortcuts_open = false;
    }
    if context.text_edit_focused() {
        return;
    }
    const DIGITS: [Key; 7] = [
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
    ];
    for (key, page) in DIGITS.into_iter().zip(Page::NAVIGATION) {
        if context
            .input_mut(|input| input.consume_shortcut(&KeyboardShortcut::new(Modifiers::ALT, key)))
        {
            workbench.go(page);
        }
    }
    if context.input_mut(|input| input.consume_key(Modifiers::SHIFT, Key::Slash))
        || context.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Questionmark))
    {
        workbench.shortcuts_open = !workbench.shortcuts_open;
    }
}

/// Every shortcut, grouped by where it works.
const SHEET: [(&str, &[(&str, &str)]); 3] = [
    (
        "Anywhere",
        &[
            ("Alt+1 … Alt+7", "Open a page of the navigation"),
            ("?", "Show or hide this sheet"),
            ("Esc", "Close this sheet or the side panel"),
        ],
    ),
    (
        "Editor",
        &[
            ("Ctrl+S", "Save the draft"),
            ("Ctrl+Enter", "Run the published workflow"),
            ("Ctrl+Z", "Undo"),
            ("Ctrl+Shift+Z or Ctrl+Y", "Redo"),
            ("Delete", "Remove the selected node"),
        ],
    ),
    (
        "Lists",
        &[
            ("Tab / Shift+Tab", "Move between rows and controls"),
            ("Enter or Space", "Open or press the focused item"),
        ],
    ),
];

/// The shortcut sheet, a window over the page.
pub(crate) fn sheet(context: &egui::Context, workbench: &mut Workbench) {
    if !workbench.shortcuts_open {
        return;
    }
    let mut open = true;
    egui::Window::new("Keyboard shortcuts")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(theme::card())
        .show(context, |ui| {
            let room = theme::SPACE_XL.mul_add(-2.0, context.content_rect().width());
            ui.set_width(420.0_f32.min(room));
            for (group, entries) in SHEET {
                widgets::section(ui, group);
                // One key column width for every group, so the actions line up down the sheet.
                egui::Grid::new(("shortcuts", group))
                    .num_columns(2)
                    .min_col_width(170.0)
                    .spacing([theme::SPACE_LG, theme::SPACE_SM])
                    .show(ui, |ui| {
                        for (keys, action) in entries {
                            ui.label(RichText::new(*keys).monospace().color(theme::ACCENT));
                            ui.label(*action);
                            ui.end_row();
                        }
                    });
                ui.add_space(theme::SPACE_MD);
            }
        });
    workbench.shortcuts_open = open;
}
