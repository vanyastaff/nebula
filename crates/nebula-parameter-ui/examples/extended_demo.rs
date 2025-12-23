use chrono::{Duration, Utc};
use eframe::egui;
use nebula_parameter::{
    ExpirableParameter, ExpirableParameterOptions, ExpirableValue, HiddenParameter, ModeItem,
    ModeParameter, ModeValue, NoticeParameter, NoticeParameterOptions, NoticeType,
    ParameterDisplay, ParameterMetadata,
};
use nebula_parameter_ui::{
    ExpirableWidget, HiddenWidget, ModeWidget, NoticeWidget, ParameterTheme, ParameterWidget,
};
use nebula_value::Text;

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        initial_window_size: Some(egui::Vec2::new(1000.0, 800.0)),
        ..Default::default()
    };

    eframe::run_native(
        "Nebula Parameter UI - Extended Demo",
        options,
        Box::new(|_cc| Box::new(ExtendedDemoApp::new())),
    )
}

struct ExtendedDemoApp {
    theme: ParameterTheme,
    use_dark_theme: bool,

    // Notice widgets
    info_notice: NoticeWidget,
    warning_notice: NoticeWidget,
    error_notice: NoticeWidget,
    success_notice: NoticeWidget,

    // Hidden widget
    hidden_widget: HiddenWidget,

    // Mode widget
    mode_widget: ModeWidget,

    // Expirable widget
    expirable_widget: ExpirableWidget,
}

impl ExtendedDemoApp {
    fn new() -> Self {
        // Create notice widgets
        let info_notice = NoticeWidget::new(NoticeParameter {
            metadata: ParameterMetadata {
                key: "info_notice".to_string(),
                name: "Information Notice".to_string(),
                description: Some(
                    "This is an informational notice with dismissible option.".to_string(),
                ),
                required: false,
            },
            content: "This is an informational message that provides helpful context to users."
                .to_string(),
            options: Some(NoticeParameterOptions {
                notice_type: Some(NoticeType::Info),
                dismissible: true,
            }),
            display: None,
        });

        let warning_notice = NoticeWidget::new(NoticeParameter {
            metadata: ParameterMetadata {
                key: "warning_notice".to_string(),
                name: "Warning Notice".to_string(),
                description: None,
                required: false,
            },
            content: "Warning: This operation may have unintended consequences. Please review before proceeding.".to_string(),
            options: Some(NoticeParameterOptions {
                notice_type: Some(NoticeType::Warning),
                dismissible: false,
            }),
            display: None,
        });

        let error_notice = NoticeWidget::new(NoticeParameter {
            metadata: ParameterMetadata {
                key: "error_notice".to_string(),
                name: "Error Notice".to_string(),
                description: None,
                required: false,
            },
            content:
                "Error: Failed to connect to the database. Please check your connection settings."
                    .to_string(),
            options: Some(NoticeParameterOptions {
                notice_type: Some(NoticeType::Error),
                dismissible: true,
            }),
            display: None,
        });

        let success_notice = NoticeWidget::new(NoticeParameter {
            metadata: ParameterMetadata {
                key: "success_notice".to_string(),
                name: "Success Notice".to_string(),
                description: None,
                required: false,
            },
            content: "Success: Your data has been saved successfully!".to_string(),
            options: Some(NoticeParameterOptions {
                notice_type: Some(NoticeType::Success),
                dismissible: true,
            }),
            display: None,
        });

        // Create hidden widget
        let hidden_widget = HiddenWidget::new(HiddenParameter {
            metadata: ParameterMetadata {
                key: "hidden_param".to_string(),
                name: "Hidden Parameter".to_string(),
                description: Some(
                    "This parameter is not visible in the UI but stores data.".to_string(),
                ),
                required: false,
            },
            value: Some(Text::new("hidden_value")),
        });

        // Create mode widget
        let mode_widget = ModeWidget::new(ModeParameter {
            metadata: ParameterMetadata {
                key: "mode_selection".to_string(),
                name: "Mode Selection".to_string(),
                description: Some(
                    "Select a mode to configure different parameter sets.".to_string(),
                ),
                required: true,
            },
            value: None,
            default: None,
            modes: vec![
                ModeItem {
                    key: "basic".to_string(),
                    name: "Basic Mode".to_string(),
                    description: Some("Simple configuration with basic parameters.".to_string()),
                    parameters: vec![],
                },
                ModeItem {
                    key: "advanced".to_string(),
                    name: "Advanced Mode".to_string(),
                    description: Some(
                        "Advanced configuration with all available options.".to_string(),
                    ),
                    parameters: vec![],
                },
                ModeItem {
                    key: "expert".to_string(),
                    name: "Expert Mode".to_string(),
                    description: Some(
                        "Expert configuration with full control over all settings.".to_string(),
                    ),
                    parameters: vec![],
                },
            ],
            display: None,
            validation: None,
        });

        // Create expirable widget
        let now = Utc::now();
        let expires_at = now + Duration::seconds(300); // 5 minutes
        let expirable_value = ExpirableValue::new(
            Text::new("This value will expire in 5 minutes"),
            now,
            expires_at,
        );

        let expirable_widget = ExpirableWidget::new(ExpirableParameter {
            metadata: ParameterMetadata {
                key: "expirable_param".to_string(),
                name: "Expirable Parameter".to_string(),
                description: Some(
                    "This parameter has a Time-To-Live and will expire automatically.".to_string(),
                ),
                required: false,
            },
            value: Some(expirable_value),
            default: None,
            options: Some(ExpirableParameterOptions {
                ttl: 300, // 5 minutes
                auto_refresh: false,
                refresh_on_access: true,
            }),
            display: None,
            validation: None,
            children: None,
        });

        Self {
            theme: ParameterTheme::dark(),
            use_dark_theme: true,
            info_notice,
            warning_notice,
            error_notice,
            success_notice,
            hidden_widget,
            mode_widget,
            expirable_widget,
        }
    }
}

impl eframe::App for ExtendedDemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Theme toggle
        egui::TopBottomPanel::top("theme_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Theme:");
                if ui.button("🌙 Dark").clicked() {
                    self.use_dark_theme = true;
                    self.theme = ParameterTheme::dark();
                }
                if ui.button("☀️ Light").clicked() {
                    self.use_dark_theme = false;
                    self.theme = ParameterTheme::light();
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ctx, |ui| {
                ui.add_space(10.0);

                // Notice Widgets
                ui.heading("📢 Notice Widgets");
                ui.separator();

                ui.group(|ui| {
                    self.info_notice.render_with_theme(ui, &self.theme);
                });

                ui.add_space(8.0);

                ui.group(|ui| {
                    self.warning_notice.render_with_theme(ui, &self.theme);
                });

                ui.add_space(8.0);

                ui.group(|ui| {
                    self.error_notice.render_with_theme(ui, &self.theme);
                });

                ui.add_space(8.0);

                ui.group(|ui| {
                    self.success_notice.render_with_theme(ui, &self.theme);
                });

                ui.add_space(20.0);

                // Hidden Widget
                ui.heading("👻 Hidden Widget");
                ui.separator();

                ui.label("The hidden widget below is invisible but stores data:");
                ui.add_space(4.0);
                self.hidden_widget.render_with_theme(ui, &self.theme);
                ui.label("(Hidden widget rendered above - you can't see it!)");

                ui.add_space(20.0);

                // Mode Widget
                ui.heading("🔄 Mode Widget");
                ui.separator();

                ui.group(|ui| {
                    self.mode_widget.render_with_theme(ui, &self.theme);
                });

                ui.add_space(20.0);

                // Expirable Widget
                ui.heading("⏰ Expirable Widget");
                ui.separator();

                ui.group(|ui| {
                    self.expirable_widget.render_with_theme(ui, &self.theme);
                });

                ui.add_space(20.0);

                // Information
                ui.heading("ℹ️ Information");
                ui.separator();
                
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("This demo showcases the extended parameter widgets:");
                        ui.add_space(4.0);
                        ui.label("• Notice Widget: Display informational messages with different types");
                        ui.label("• Hidden Widget: Invisible parameters for internal data storage");
                        ui.label("• Mode Widget: Dynamic mode selection with parameter switching");
                        ui.label("• Expirable Widget: Parameters with Time-To-Live expiration");
                        ui.add_space(8.0);
                        ui.label("These widgets extend the basic parameter functionality with specialized features for complex applications.");
                    });
                });

                ui.add_space(20.0);
            });
        });
    }
}
