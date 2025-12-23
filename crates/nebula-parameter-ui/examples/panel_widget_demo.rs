//! Demo application showcasing PanelWidget
//!
//! Run with: cargo run -p nebula-parameter-ui --example panel_widget_demo

use eframe::egui;
use nebula_parameter::core::ParameterMetadata;
use nebula_parameter::types::{
    CheckboxParameter, Panel, PanelParameter, PanelParameterOptions, TextParameter,
};
use nebula_parameter_ui::{PanelWidget, ParameterTheme, ParameterWidget};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 500.0])
            .with_title("PanelWidget Demo"),
        ..Default::default()
    };

    eframe::run_native(
        "PanelWidget Demo",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(DemoApp::new()))
        }),
    )
}

struct DemoApp {
    theme: ParameterTheme,
    panel_tabs: PanelWidget,
    panel_accordion: PanelWidget,
}

impl DemoApp {
    fn new() -> Self {
        Self {
            theme: ParameterTheme::dark(),
            panel_tabs: PanelWidget::new(create_tabs_panel()),
            panel_accordion: PanelWidget::new(create_accordion_panel()),
        }
    }
}

impl eframe::App for DemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("PanelWidget Demo");
                ui.separator();

                // Theme switcher
                ui.horizontal(|ui| {
                    ui.label("Theme:");
                    if ui.button("Light").clicked() {
                        self.theme = ParameterTheme::light();
                        ctx.set_visuals(egui::Visuals::light());
                    }
                    if ui.button("Dark").clicked() {
                        self.theme = ParameterTheme::dark();
                        ctx.set_visuals(egui::Visuals::dark());
                    }
                });

                ui.add_space(16.0);

                // Tabs mode panel
                ui.heading("Tabs Mode (single panel open)");
                egui::Frame::none()
                    .fill(ui.visuals().faint_bg_color)
                    .rounding(8.0)
                    .inner_margin(16.0)
                    .show(ui, |ui| {
                        let response = self.panel_tabs.show(ui, &self.theme);
                        if response.changed {
                            ui.label(
                                egui::RichText::new("Panel changed!").color(egui::Color32::GREEN),
                            );
                        }
                    });

                ui.add_space(24.0);

                // Accordion mode panel
                ui.heading("Accordion Mode (multiple panels open)");
                egui::Frame::none()
                    .fill(ui.visuals().faint_bg_color)
                    .rounding(8.0)
                    .inner_margin(16.0)
                    .show(ui, |ui| {
                        let response = self.panel_accordion.show(ui, &self.theme);
                        if response.changed {
                            ui.label(
                                egui::RichText::new("Panel changed!").color(egui::Color32::GREEN),
                            );
                        }
                    });

                ui.add_space(16.0);

                // Show current state
                ui.separator();
                ui.heading("Current State");
                if let Some(active) = self.panel_tabs.active_panel() {
                    ui.label(format!("Tabs - Active panel: {}", active));
                }
                ui.label(format!(
                    "Accordion - General open: {}",
                    self.panel_accordion.is_panel_open("general")
                ));
                ui.label(format!(
                    "Accordion - Security open: {}",
                    self.panel_accordion.is_panel_open("security")
                ));
                ui.label(format!(
                    "Accordion - Notifications open: {}",
                    self.panel_accordion.is_panel_open("notifications")
                ));
            });
        });
    }
}

fn create_metadata(key: &str, name: &str, description: &str) -> ParameterMetadata {
    ParameterMetadata::builder()
        .key(key)
        .name(name)
        .description(description)
        .build()
        .unwrap()
}

/// Create a panel parameter in tabs mode (only one panel open at a time)
fn create_tabs_panel() -> PanelParameter {
    // Create child parameters for panels
    let general_name = TextParameter::builder()
        .metadata(create_metadata("name", "Name", "Enter your name"))
        .build();

    let general_email = TextParameter::builder()
        .metadata(create_metadata("email", "Email", "Enter your email"))
        .build();

    let security_enabled = CheckboxParameter::builder()
        .metadata(create_metadata(
            "security_enabled",
            "Enable Security",
            "Turn on security features",
        ))
        .build();

    let notifications_email = CheckboxParameter::builder()
        .metadata(create_metadata(
            "notify_email",
            "Email Notifications",
            "Receive email alerts",
        ))
        .build();

    // Create panels
    let general_panel = Panel::new("general", "General")
        .with_description("Basic account settings")
        .with_icon("⚙")
        .with_child(Box::new(general_name))
        .with_child(Box::new(general_email));

    let security_panel = Panel::new("security", "Security")
        .with_description("Security settings")
        .with_icon("🔒")
        .with_child(Box::new(security_enabled));

    let notifications_panel = Panel::new("notifications", "Notifications")
        .with_description("Notification preferences")
        .with_icon("🔔")
        .with_child(Box::new(notifications_email));

    let mut panel_param = PanelParameter::new(create_metadata(
        "settings_tabs",
        "Account Settings (Tabs)",
        "Configure your account using tabs",
    ));

    // Set options - single panel mode (tabs)
    panel_param.options = Some(
        PanelParameterOptions::builder()
            .default_panel("general".to_string())
            .allow_multiple_open(false)
            .build(),
    );

    panel_param.add_panel(general_panel);
    panel_param.add_panel(security_panel);
    panel_param.add_panel(notifications_panel);

    panel_param
}

/// Create a panel parameter in accordion mode (multiple panels can be open)
fn create_accordion_panel() -> PanelParameter {
    // Create child parameters
    let profile_bio = TextParameter::builder()
        .metadata(create_metadata(
            "bio",
            "Biography",
            "Tell us about yourself",
        ))
        .build();

    let privacy_public = CheckboxParameter::builder()
        .metadata(create_metadata(
            "public_profile",
            "Public Profile",
            "Make profile visible",
        ))
        .build();

    let advanced_debug = CheckboxParameter::builder()
        .metadata(create_metadata(
            "debug_mode",
            "Debug Mode",
            "Enable debug logging",
        ))
        .build();

    // Create panels
    let profile_panel = Panel::new("general", "Profile")
        .with_description("Your public profile information")
        .with_icon("👤")
        .with_child(Box::new(profile_bio));

    let privacy_panel = Panel::new("security", "Privacy")
        .with_description("Control who can see your data")
        .with_icon("🔐")
        .with_child(Box::new(privacy_public));

    let advanced_panel = Panel::new("notifications", "Advanced")
        .with_description("Advanced settings for power users")
        .with_icon("⚡")
        .with_child(Box::new(advanced_debug));

    let mut panel_param = PanelParameter::new(create_metadata(
        "settings_accordion",
        "Settings (Accordion)",
        "Expand multiple sections at once",
    ));

    // Set options - multiple panels mode (accordion)
    panel_param.options = Some(
        PanelParameterOptions::builder()
            .allow_multiple_open(true)
            .build(),
    );

    panel_param.add_panel(profile_panel);
    panel_param.add_panel(privacy_panel);
    panel_param.add_panel(advanced_panel);

    panel_param
}
