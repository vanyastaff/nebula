//! Top bar and toasts shared by every layout.
use crate::{
    theme,
    widgets::{self, Tone},
    workbench::Workbench,
};
use eframe::egui::{self, Align, Layout, RichText};

/// Brand and location on the left, account on the right. Phones wrap the account below the brand, so
/// nothing scrolls sideways.
pub(crate) fn header(ui: &mut egui::Ui, workbench: &mut Workbench, wide: bool) {
    if wide {
        ui.horizontal(|ui| {
            location(ui, workbench, wide);
            // A right-to-left layout puts the first item at the right edge, so it gets the items reversed.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                for item in ACCOUNT.iter().rev() {
                    account_item(ui, workbench, *item);
                }
            });
        });
    } else {
        ui.horizontal_wrapped(|ui| {
            location(ui, workbench, wide);
            for item in ACCOUNT {
                account_item(ui, workbench, item);
            }
        });
    }
}

#[derive(Clone, Copy)]
enum AccountItem {
    /// A spinner while a request is in flight, so work in progress never needs a message.
    Activity,
    Email,
    SwitchWorkspace,
    SignOut,
}

/// Account entries in reading order.
const ACCOUNT: [AccountItem; 4] = [
    AccountItem::Activity,
    AccountItem::Email,
    AccountItem::SwitchWorkspace,
    AccountItem::SignOut,
];

/// How long an informational toast stays. Failures stay until dismissed.
const TOAST_SECONDS: f64 = 4.0;
const TOAST_WIDTH: f32 = 360.0;

fn brand(ui: &mut egui::Ui) {
    widgets::mark(ui, "N", theme::ACCENT, 24.0);
    ui.label(RichText::new("Nebula").size(17.0).strong());
}

fn location(ui: &mut egui::Ui, workbench: &mut Workbench, wide: bool) {
    brand(ui);
    if let Some(context) = &workbench.session.context {
        ui.separator();
        ui.label(RichText::new(&context.organization).color(theme::TEXT_MUTED));
        widgets::caption(ui, "/");
        ui.label(RichText::new(&context.workspace_selector).strong());
    }
    // Phones have no room for the sidebar beside the page, so its list opens from here instead.
    if !wide && workbench.workspace_open() && !workbench.workspace_form_open {
        let open = egui::Button::new("Workflows").selected(workbench.sidebar_open);
        if ui.add(open).clicked() {
            workbench.sidebar_open = !workbench.sidebar_open;
        }
    }
}

fn account_item(ui: &mut egui::Ui, workbench: &mut Workbench, item: AccountItem) {
    match item {
        AccountItem::Activity => {
            if workbench.session.busy() {
                ui.spinner();
            }
        },
        AccountItem::Email => {
            if let Some(profile) = &workbench.profile {
                widgets::caption(ui, profile.email.as_str());
            }
        },
        AccountItem::SwitchWorkspace => {
            if workbench.workspace_open() && ui.button("Switch workspace").clicked() {
                workbench.workspace_form_open = !workbench.workspace_form_open;
            }
        },
        AccountItem::SignOut => {
            if workbench.is_signed_in() && ui.button("Sign out").clicked() {
                workbench.disconnect();
            }
        },
    }
}

/// The outcome of the last action as a toast in the bottom-right corner. Information fades after a few
/// seconds; a failure stays until the user dismisses it, so it cannot be missed.
pub(crate) fn toast(context: &egui::Context, workbench: &mut Workbench) {
    if workbench.feedback.message.is_empty() {
        return;
    }
    let now = context.input(|input| input.time);
    let serial = workbench.feedback.serial;
    // The clock of a message starts on the first frame that shows it.
    let shown = context.data_mut(|data| {
        let clock = data.get_temp_mut_or(egui::Id::new("toast-clock"), (serial, now));
        if clock.0 != serial {
            *clock = (serial, now);
        }
        clock.1
    });
    let failure = workbench.feedback.failure;
    if !failure {
        let left = TOAST_SECONDS - (now - shown);
        if left <= 0.0 {
            return;
        }
        context.request_repaint_after(std::time::Duration::from_secs_f64(left));
    }
    let tone = if failure { Tone::Danger } else { Tone::Neutral };
    egui::Area::new(egui::Id::new("toast"))
        .anchor(
            egui::Align2::RIGHT_BOTTOM,
            [-theme::SPACE_LG, -theme::SPACE_LG],
        )
        .order(egui::Order::Foreground)
        .show(context, |ui| {
            ui.set_max_width(TOAST_WIDTH);
            widgets::banner(ui, tone, &workbench.feedback.message);
            if failure
                && ui
                    .with_layout(Layout::right_to_left(Align::Min), |ui| {
                        ui.small_button("Dismiss").clicked()
                    })
                    .inner
            {
                workbench.feedback.dismiss();
            }
        });
}
