//! Demo application to test all parameter widgets.
//!
//! Run with: cargo run -p nebula-parameter-ui --example all_widgets_demo

use eframe::egui;
use nebula_parameter::SelectOption;
use nebula_parameter::core::ParameterMetadata;
use nebula_parameter::types::*;
use nebula_parameter_ui::*;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 900.0])
            .with_title("Parameter Widgets Demo"),
        ..Default::default()
    };

    eframe::run_native(
        "Parameter Widgets Demo",
        options,
        Box::new(|cc| {
            // Use dark theme
            cc.egui_ctx.set_visuals(egui::Visuals::dark());

            // Register phosphor icon fonts
            let mut fonts = egui::FontDefinitions::default();
            egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
            cc.egui_ctx.set_fonts(fonts);

            Ok(Box::new(DemoApp::new()))
        }),
    )
}

/// Helper function to create ParameterMetadata using builder pattern
fn create_metadata(key: &str, name: &str, description: &str) -> ParameterMetadata {
    ParameterMetadata::builder()
        .key(key)
        .name(name)
        .description(description)
        .build()
        .unwrap()
}

/// Helper function to create required ParameterMetadata
fn create_required_metadata(key: &str, name: &str, description: &str) -> ParameterMetadata {
    ParameterMetadata::builder()
        .key(key)
        .name(name)
        .description(description)
        .required(true)
        .build()
        .unwrap()
}

struct DemoApp {
    theme: ParameterTheme,
    // Basic widgets
    text_widget: TextWidget,
    textarea_widget: TextareaWidget,
    checkbox_widget: CheckboxWidget,
    secret_widget: SecretWidget,
    // Number widgets
    number_basic: NumberWidget,
    number_range: NumberWidget,
    number_step: NumberWidget,
    number_precision: NumberWidget,
    // Selection widgets
    select_widget: SelectWidget,
    radio_widget: RadioWidget,
    multi_select_widget: MultiSelectWidget,
    // Date/time widgets
    date_widget: DateWidget,
    time_widget: TimeWidget,
    datetime_widget: DateTimeWidget,
    // Special widgets
    color_widget: ColorWidget,
    code_widget: CodeWidget,
    notice_info: NoticeWidget,
    notice_warning: NoticeWidget,
    notice_error: NoticeWidget,
    notice_success: NoticeWidget,
    // Container widgets
    list_widget: ListWidget,
    object_widget: ObjectWidget,
    // Mode widget
    mode_widget: ModeWidget,
    // Panel widget
    panel_widget: PanelWidget,
}

impl DemoApp {
    fn new() -> Self {
        Self {
            theme: ParameterTheme::dark(),
            text_widget: TextWidget::new(create_text_parameter()),
            textarea_widget: TextareaWidget::new(create_textarea_parameter()),
            checkbox_widget: CheckboxWidget::new(create_checkbox_parameter()),
            secret_widget: SecretWidget::new(create_secret_parameter()),
            // Number widgets with different configurations
            number_basic: NumberWidget::new(create_number_basic()),
            number_range: NumberWidget::new(create_number_range()),
            number_step: NumberWidget::new(create_number_step()),
            number_precision: NumberWidget::new(create_number_precision()),
            select_widget: SelectWidget::new(create_select_parameter()),
            radio_widget: RadioWidget::new(create_radio_parameter()),
            multi_select_widget: MultiSelectWidget::new(create_multi_select_parameter()),
            date_widget: DateWidget::new(create_date_parameter()),
            time_widget: TimeWidget::new(create_time_parameter()),
            datetime_widget: DateTimeWidget::new(create_datetime_parameter()),
            color_widget: ColorWidget::new(create_color_parameter()),
            code_widget: CodeWidget::new(create_code_parameter()),
            notice_info: NoticeWidget::new(create_notice_parameter(
                NoticeType::Info,
                "This is an info notice.",
            )),
            notice_warning: NoticeWidget::new(create_notice_parameter(
                NoticeType::Warning,
                "This is a warning notice.",
            )),
            notice_error: NoticeWidget::new(create_notice_parameter(
                NoticeType::Error,
                "This is an error notice.",
            )),
            notice_success: NoticeWidget::new(create_notice_parameter(
                NoticeType::Success,
                "This is a success notice.",
            )),
            list_widget: ListWidget::new(create_list_parameter()),
            object_widget: ObjectWidget::new(create_object_parameter()),
            mode_widget: ModeWidget::new(create_mode_parameter()),
            panel_widget: PanelWidget::new(create_panel_parameter()),
        }
    }
}

impl eframe::App for DemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Navigation panel
        egui::SidePanel::left("nav_panel")
            .min_width(150.0)
            .show(ctx, |ui| {
                ui.heading("Widgets");
                ui.separator();

                ui.label("Basic:");
                ui.small("  Text, Textarea, Number");
                ui.small("  Checkbox, Secret");
                ui.separator();
                ui.label("Selection:");
                ui.small("  Select, Radio, MultiSelect");
                ui.separator();
                ui.label("Date/Time:");
                ui.small("  Date, Time, DateTime");
                ui.separator();
                ui.label("Special:");
                ui.small("  Color, Code, Notice");
                ui.separator();
                ui.label("Container:");
                ui.small("  List, Object, Mode, Panel");

                ui.separator();
                ui.heading("Theme");
                if ui.button("Light").clicked() {
                    self.theme = ParameterTheme::light();
                    ctx.set_visuals(egui::Visuals::light());
                }
                if ui.button("Dark").clicked() {
                    self.theme = ParameterTheme::dark();
                    ctx.set_visuals(egui::Visuals::dark());
                }
            });

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Parameter Widgets Demo");
                ui.label("Testing all nebula-parameter-ui widgets");
                ui.separator();

                // Basic Widgets Section
                ui.heading("Basic Widgets");
                ui.add_space(8.0);

                // Text
                widget_section(ui, "TextWidget", |ui| {
                    let response = self.text_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                // Textarea
                widget_section(ui, "TextareaWidget", |ui| {
                    let response = self.textarea_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                // Number - Different configurations
                widget_section(ui, "NumberWidget - Various Configurations", |ui| {
                    ui.label("Basic number input:");
                    let response = self.number_basic.show(ui, &self.theme);
                    show_response(ui, &response);

                    ui.add_space(12.0);
                    ui.label("With range (0-1000):");
                    let response = self.number_range.show(ui, &self.theme);
                    show_response(ui, &response);

                    ui.add_space(12.0);
                    ui.label("With step (increment by 5):");
                    let response = self.number_step.show(ui, &self.theme);
                    show_response(ui, &response);

                    ui.add_space(12.0);
                    ui.label("With precision (2 decimal places):");
                    let response = self.number_precision.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                // Checkbox
                widget_section(ui, "CheckboxWidget", |ui| {
                    let response = self.checkbox_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                // Secret
                widget_section(ui, "SecretWidget", |ui| {
                    let response = self.secret_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                ui.separator();
                ui.heading("Selection Widgets");
                ui.add_space(8.0);

                // Select
                widget_section(ui, "SelectWidget", |ui| {
                    let response = self.select_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                // Radio
                widget_section(ui, "RadioWidget", |ui| {
                    let response = self.radio_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                // MultiSelect
                widget_section(ui, "MultiSelectWidget", |ui| {
                    let response = self.multi_select_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                ui.separator();
                ui.heading("Date/Time Widgets");
                ui.add_space(8.0);

                // Date
                widget_section(ui, "DateWidget", |ui| {
                    let response = self.date_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                // Time
                widget_section(ui, "TimeWidget", |ui| {
                    let response = self.time_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                // DateTime
                widget_section(ui, "DateTimeWidget", |ui| {
                    let response = self.datetime_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                ui.separator();
                ui.heading("Special Widgets");
                ui.add_space(8.0);

                // Color
                widget_section(ui, "ColorWidget", |ui| {
                    let response = self.color_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                // Code
                widget_section(ui, "CodeWidget", |ui| {
                    let response = self.code_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                // Notice - all types
                widget_section(ui, "NoticeWidget (all types)", |ui| {
                    self.notice_info.show(ui, &self.theme);
                    ui.add_space(4.0);
                    self.notice_warning.show(ui, &self.theme);
                    ui.add_space(4.0);
                    self.notice_error.show(ui, &self.theme);
                    ui.add_space(4.0);
                    self.notice_success.show(ui, &self.theme);
                });

                ui.separator();
                ui.heading("Container Widgets");
                ui.add_space(8.0);

                // List
                widget_section(ui, "ListWidget", |ui| {
                    let response = self.list_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                // Object
                widget_section(ui, "ObjectWidget", |ui| {
                    let response = self.object_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                // Mode
                widget_section(ui, "ModeWidget", |ui| {
                    let response = self.mode_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                // Panel
                widget_section(ui, "PanelWidget", |ui| {
                    let response = self.panel_widget.show(ui, &self.theme);
                    show_response(ui, &response);
                });

                ui.add_space(50.0);
            });
        });
    }
}

fn widget_section(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(8.0)
        .inner_margin(16.0)
        .outer_margin(egui::Margin::symmetric(0, 4))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(title).strong().size(14.0));
            ui.add_space(8.0);
            content(ui);
        });
}

fn show_response(ui: &mut egui::Ui, response: &WidgetResponse) {
    ui.horizontal(|ui| {
        if response.changed {
            ui.colored_label(egui::Color32::GREEN, "Changed!");
        }
        if response.lost_focus {
            ui.colored_label(egui::Color32::YELLOW, "Lost focus");
        }
        if let Some(ref error) = response.error {
            ui.colored_label(egui::Color32::RED, format!("Error: {}", error));
        }
    });
}

// Parameter creation helpers

fn create_text_parameter() -> TextParameter {
    TextParameter::builder()
        .metadata(create_metadata(
            "username",
            "Username",
            "Enter your username",
        ))
        .build()
}

fn create_textarea_parameter() -> TextareaParameter {
    TextareaParameter::builder()
        .metadata(create_metadata(
            "bio",
            "Biography",
            "Tell us about yourself",
        ))
        .build()
}

fn create_number_basic() -> NumberParameter {
    NumberParameter::builder()
        .metadata(create_metadata("quantity", "Quantity", "Enter a number"))
        .build()
}

fn create_number_range() -> NumberParameter {
    NumberParameter::builder()
        .metadata(create_metadata(
            "price",
            "Price",
            "Value between 0 and 1000",
        ))
        .options(
            NumberParameterOptions::builder()
                .min(0.0)
                .max(1000.0)
                .build(),
        )
        .build()
}

fn create_number_step() -> NumberParameter {
    NumberParameter::builder()
        .metadata(create_metadata("count", "Count", "Increment by 5"))
        .options(
            NumberParameterOptions::builder()
                .min(0.0)
                .max(100.0)
                .step(5.0)
                .build(),
        )
        .build()
}

fn create_number_precision() -> NumberParameter {
    NumberParameter::builder()
        .metadata(create_metadata(
            "amount",
            "Amount",
            "2 decimal places allowed",
        ))
        .options(
            NumberParameterOptions::builder()
                .min(0.0)
                .max(100.0)
                .precision(2)
                .build(),
        )
        .build()
}

fn create_checkbox_parameter() -> CheckboxParameter {
    CheckboxParameter::builder()
        .metadata(create_metadata(
            "agree",
            "I agree to terms",
            "You must agree to continue",
        ))
        .build()
}

fn create_secret_parameter() -> SecretParameter {
    SecretParameter::builder()
        .metadata(create_metadata(
            "password",
            "Password",
            "Enter a secure password",
        ))
        .build()
}

fn create_select_parameter() -> SelectParameter {
    SelectParameter::builder()
        .metadata(create_metadata("country", "Country", "Select your country"))
        .options(vec![
            SelectOption::new("us", "United States", "us"),
            SelectOption::new("uk", "United Kingdom", "uk"),
            SelectOption::new("de", "Germany", "de"),
            SelectOption::new("fr", "France", "fr"),
            SelectOption::new("jp", "Japan", "jp"),
        ])
        .build()
}

fn create_radio_parameter() -> RadioParameter {
    RadioParameter::builder()
        .metadata(create_metadata(
            "plan",
            "Subscription Plan",
            "Choose your plan",
        ))
        .options(vec![
            SelectOption::new("free", "Free", "free"),
            SelectOption::new("basic", "Basic - $9/mo", "basic"),
            SelectOption::new("pro", "Pro - $29/mo", "pro"),
        ])
        .build()
}

fn create_multi_select_parameter() -> MultiSelectParameter {
    MultiSelectParameter::builder()
        .metadata(create_metadata(
            "interests",
            "Interests",
            "Select your interests",
        ))
        .options(vec![
            SelectOption::new("tech", "Technology", "tech"),
            SelectOption::new("music", "Music", "music"),
            SelectOption::new("sports", "Sports", "sports"),
            SelectOption::new("travel", "Travel", "travel"),
            SelectOption::new("food", "Food & Cooking", "food"),
        ])
        .build()
}

fn create_date_parameter() -> DateParameter {
    DateParameter::builder()
        .metadata(create_metadata(
            "birthday",
            "Birthday",
            "Enter your date of birth",
        ))
        .build()
}

fn create_time_parameter() -> TimeParameter {
    TimeParameter::builder()
        .metadata(create_metadata("alarm", "Alarm Time", "Set your alarm"))
        .build()
}

fn create_datetime_parameter() -> DateTimeParameter {
    DateTimeParameter::builder()
        .metadata(create_metadata(
            "meeting",
            "Meeting Time",
            "Schedule your meeting",
        ))
        .build()
}

fn create_color_parameter() -> ColorParameter {
    ColorParameter::builder()
        .metadata(create_metadata(
            "theme_color",
            "Theme Color",
            "Choose your theme color",
        ))
        .build()
}

fn create_code_parameter() -> CodeParameter {
    CodeParameter::builder()
        .metadata(create_metadata("script", "Script", "Enter your code"))
        .build()
}

fn create_notice_parameter(notice_type: NoticeType, message: &str) -> NoticeParameter {
    NoticeParameter::builder()
        .metadata(create_metadata("notice", "Notice", ""))
        .content(message.to_string())
        .options(
            NoticeParameterOptions::builder()
                .notice_type(notice_type)
                .build(),
        )
        .build()
}

fn create_list_parameter() -> ListParameter {
    ListParameter::new("tags", "Tags", "Add tags to your item").unwrap()
}

fn create_object_parameter() -> ObjectParameter {
    let mut obj =
        ObjectParameter::new("user_settings", "User Settings", "Configure user options").unwrap();

    // Required parameters (always shown)
    let name_param = TextParameter::builder()
        .metadata(create_required_metadata(
            "name",
            "Full Name",
            "Enter your full name",
        ))
        .build();

    let email_param = TextParameter::builder()
        .metadata(create_required_metadata(
            "email",
            "Email",
            "Enter your email address",
        ))
        .build();

    // Optional parameters (can be added via "Add Parameter" button)
    let age_param = NumberParameter::builder()
        .metadata(create_metadata("age", "Age", "Enter your age"))
        .options(
            NumberParameterOptions::builder()
                .min(0.0)
                .max(150.0)
                .precision(0)
                .build(),
        )
        .build();

    let phone_param = TextParameter::builder()
        .metadata(create_metadata(
            "phone",
            "Phone Number",
            "Enter phone number",
        ))
        .build();

    let website_param = TextParameter::builder()
        .metadata(create_metadata(
            "website",
            "Website",
            "Your personal website",
        ))
        .build();

    let newsletter_param = CheckboxParameter::builder()
        .metadata(create_metadata(
            "newsletter",
            "Subscribe to Newsletter",
            "Get updates",
        ))
        .build();

    let bio_param = TextParameter::builder()
        .metadata(create_metadata(
            "bio",
            "Biography",
            "Tell us about yourself",
        ))
        .build();

    // Add required first
    obj.add_child("name", Box::new(name_param));
    obj.add_child("email", Box::new(email_param));

    // Add optional
    obj.add_child("age", Box::new(age_param));
    obj.add_child("phone", Box::new(phone_param));
    obj.add_child("website", Box::new(website_param));
    obj.add_child("newsletter", Box::new(newsletter_param));
    obj.add_child("bio", Box::new(bio_param));

    obj
}

fn create_mode_parameter() -> ModeParameter {
    // Create child parameters for each mode
    let email_param = TextParameter::builder()
        .metadata(create_metadata(
            "email",
            "Email Address",
            "Enter recipient email",
        ))
        .build();

    let sms_param = TextParameter::builder()
        .metadata(create_metadata(
            "phone",
            "Phone Number",
            "Enter phone number",
        ))
        .build();

    let webhook_param = TextParameter::builder()
        .metadata(create_metadata(
            "url",
            "Webhook URL",
            "Enter webhook endpoint",
        ))
        .build();

    let mut mode_param = ModeParameter::new(
        "notification_mode",
        "Notification Method",
        "Choose how to send notifications",
    )
    .unwrap();

    mode_param.add_mode(ModeItem {
        key: "email".to_string(),
        name: "Email".to_string(),
        description: Some("Send notifications via email".to_string()),
        children: Box::new(email_param),
        default: true,
    });

    mode_param.add_mode(ModeItem {
        key: "sms".to_string(),
        name: "SMS".to_string(),
        description: Some("Send notifications via SMS".to_string()),
        children: Box::new(sms_param),
        default: false,
    });

    mode_param.add_mode(ModeItem {
        key: "webhook".to_string(),
        name: "Webhook".to_string(),
        description: Some("Send notifications to a webhook endpoint".to_string()),
        children: Box::new(webhook_param),
        default: false,
    });

    mode_param
}

fn create_panel_parameter() -> PanelParameter {
    use nebula_parameter::types::Panel;

    // Create child parameters for panels
    let general_name = TextParameter::builder()
        .metadata(create_metadata("name", "Name", "Enter your name"))
        .build();

    let general_email = TextParameter::builder()
        .metadata(create_metadata("email", "Email", "Enter your email"))
        .build();

    let security_password = SecretParameter::builder()
        .metadata(create_metadata("password", "Password", "Enter password"))
        .build();

    let security_2fa = CheckboxParameter::builder()
        .metadata(create_metadata(
            "two_factor",
            "Enable 2FA",
            "Enable two-factor authentication",
        ))
        .build();

    let notifications_email = CheckboxParameter::builder()
        .metadata(create_metadata(
            "notify_email",
            "Email Notifications",
            "Receive email alerts",
        ))
        .build();

    let notifications_sms = CheckboxParameter::builder()
        .metadata(create_metadata(
            "notify_sms",
            "SMS Notifications",
            "Receive SMS alerts",
        ))
        .build();

    // Create panels
    let general_panel = Panel::new("general", "General")
        .with_description("Basic account settings")
        .with_icon("G")
        .with_child(Box::new(general_name))
        .with_child(Box::new(general_email));

    let security_panel = Panel::new("security", "Security")
        .with_description("Security and authentication settings")
        .with_icon("S")
        .with_child(Box::new(security_password))
        .with_child(Box::new(security_2fa));

    let notifications_panel = Panel::new("notifications", "Notifications")
        .with_description("Notification preferences")
        .with_icon("N")
        .with_child(Box::new(notifications_email))
        .with_child(Box::new(notifications_sms));

    let mut panel_param = PanelParameter::new(create_metadata(
        "settings_panel",
        "Account Settings",
        "Configure your account settings using tabs",
    ));

    panel_param.add_panel(general_panel);
    panel_param.add_panel(security_panel);
    panel_param.add_panel(notifications_panel);

    panel_param
}
