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

struct DemoApp {
    theme: ParameterTheme,
    // Basic widgets
    text_widget: TextWidget,
    textarea_widget: TextareaWidget,
    checkbox_widget: CheckboxWidget,
    secret_widget: SecretWidget,
    // Number widgets - all display modes
    number_text: NumberWidget,
    number_drag: NumberWidget,
    number_slider: NumberWidget,
    number_slider_text: NumberWidget,
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
}

impl DemoApp {
    fn new() -> Self {
        Self {
            theme: ParameterTheme::dark(),
            text_widget: TextWidget::new(create_text_parameter()),
            textarea_widget: TextareaWidget::new(create_textarea_parameter()),
            checkbox_widget: CheckboxWidget::new(create_checkbox_parameter()),
            secret_widget: SecretWidget::new(create_secret_parameter()),
            // Number widgets - all display modes
            number_text: NumberWidget::new(create_number_text()),
            number_drag: NumberWidget::new(create_number_drag()),
            number_slider: NumberWidget::new(create_number_slider()),
            number_slider_text: NumberWidget::new(create_number_slider_text()),
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
                ui.small("  List");

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

                // Number - All Display Modes
                widget_section(ui, "NumberWidget - All Display Modes", |ui| {
                    ui.label("Text mode (input type=number style):");
                    let response = self.number_text.show(ui, &self.theme);
                    show_response(ui, &response);

                    ui.add_space(12.0);
                    ui.label("Drag mode (click and drag):");
                    let response = self.number_drag.show(ui, &self.theme);
                    show_response(ui, &response);

                    ui.add_space(12.0);
                    ui.label("Slider mode:");
                    let response = self.number_slider.show(ui, &self.theme);
                    show_response(ui, &response);

                    ui.add_space(12.0);
                    ui.label("SliderText mode (slider + text input):");
                    let response = self.number_slider_text.show(ui, &self.theme);
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

                ui.add_space(50.0);
            });
        });
    }
}

fn widget_section(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(ui.visuals().faint_bg_color)
        .rounding(8.0)
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
        .metadata(ParameterMetadata::new("username", "Username", "Enter your username").unwrap())
        .build()
}

fn create_textarea_parameter() -> TextareaParameter {
    TextareaParameter::builder()
        .metadata(ParameterMetadata::new("bio", "Biography", "Tell us about yourself").unwrap())
        .build()
}

fn create_number_text() -> NumberParameter {
    NumberParameter::builder()
        .metadata(
            ParameterMetadata::new("price", "Price", "Only numbers allowed, try typing letters")
                .unwrap(),
        )
        .options(
            NumberParameterOptions::new()
                .with_display_mode(NumberDisplayMode::Text)
                .with_range(0.0, 1000.0)
                .with_precision(2)
                .with_prefix("$"),
        )
        .build()
}

fn create_number_drag() -> NumberParameter {
    NumberParameter::builder()
        .metadata(
            ParameterMetadata::new("quantity", "Quantity", "Click and drag to change value")
                .unwrap(),
        )
        .options(
            NumberParameterOptions::new()
                .with_display_mode(NumberDisplayMode::Drag)
                .with_range(0.0, 100.0)
                .with_step(1.0)
                .with_precision(0)
                .with_suffix(" pcs"),
        )
        .build()
}

fn create_number_slider() -> NumberParameter {
    NumberParameter::builder()
        .metadata(ParameterMetadata::new("volume", "Volume", "Drag the slider").unwrap())
        .options(
            NumberParameterOptions::new()
                .with_display_mode(NumberDisplayMode::Slider)
                .with_range(0.0, 100.0)
                .with_suffix("%"),
        )
        .build()
}

fn create_number_slider_text() -> NumberParameter {
    NumberParameter::builder()
        .metadata(
            ParameterMetadata::new("opacity", "Opacity", "Use slider or type exact value").unwrap(),
        )
        .options(
            NumberParameterOptions::new()
                .with_display_mode(NumberDisplayMode::SliderText)
                .with_range(0.0, 100.0)
                .with_precision(1)
                .with_suffix("%"),
        )
        .build()
}

fn create_checkbox_parameter() -> CheckboxParameter {
    CheckboxParameter::builder()
        .metadata(
            ParameterMetadata::new("agree", "I agree to terms", "You must agree to continue")
                .unwrap(),
        )
        .build()
}

fn create_secret_parameter() -> SecretParameter {
    SecretParameter::builder()
        .metadata(
            ParameterMetadata::new("password", "Password", "Enter a secure password").unwrap(),
        )
        .build()
}

fn create_select_parameter() -> SelectParameter {
    SelectParameter::builder()
        .metadata(ParameterMetadata::new("country", "Country", "Select your country").unwrap())
        .options(vec![
            SelectOption::simple("us", "United States"),
            SelectOption::simple("uk", "United Kingdom"),
            SelectOption::simple("de", "Germany"),
            SelectOption::simple("fr", "France"),
            SelectOption::simple("jp", "Japan"),
        ])
        .build()
}

fn create_radio_parameter() -> RadioParameter {
    RadioParameter::builder()
        .metadata(ParameterMetadata::new("plan", "Subscription Plan", "Choose your plan").unwrap())
        .options(vec![
            SelectOption::simple("free", "Free"),
            SelectOption::simple("basic", "Basic - $9/mo"),
            SelectOption::simple("pro", "Pro - $29/mo"),
        ])
        .build()
}

fn create_multi_select_parameter() -> MultiSelectParameter {
    MultiSelectParameter::builder()
        .metadata(
            ParameterMetadata::new("interests", "Interests", "Select your interests").unwrap(),
        )
        .options(vec![
            SelectOption::simple("tech", "Technology"),
            SelectOption::simple("music", "Music"),
            SelectOption::simple("sports", "Sports"),
            SelectOption::simple("travel", "Travel"),
            SelectOption::simple("food", "Food & Cooking"),
        ])
        .build()
}

fn create_date_parameter() -> DateParameter {
    DateParameter::builder()
        .metadata(
            ParameterMetadata::new("birthday", "Birthday", "Enter your date of birth").unwrap(),
        )
        .build()
}

fn create_time_parameter() -> TimeParameter {
    TimeParameter::builder()
        .metadata(ParameterMetadata::new("alarm", "Alarm Time", "Set your alarm").unwrap())
        .build()
}

fn create_datetime_parameter() -> DateTimeParameter {
    DateTimeParameter::builder()
        .metadata(
            ParameterMetadata::new("meeting", "Meeting Time", "Schedule your meeting").unwrap(),
        )
        .build()
}

fn create_color_parameter() -> ColorParameter {
    ColorParameter::builder()
        .metadata(
            ParameterMetadata::new("theme_color", "Theme Color", "Choose your theme color")
                .unwrap(),
        )
        .build()
}

fn create_code_parameter() -> CodeParameter {
    CodeParameter::builder()
        .metadata(ParameterMetadata::new("script", "Script", "Enter your code").unwrap())
        .build()
}

fn create_notice_parameter(notice_type: NoticeType, message: &str) -> NoticeParameter {
    NoticeParameter::builder()
        .metadata(ParameterMetadata::new("notice", "Notice", "").unwrap())
        .content(message.to_string())
        .options(NoticeParameterOptions::new().with_notice_type(notice_type))
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
        .metadata(ParameterMetadata::required("name", "Full Name", "Enter your full name").unwrap())
        .build();

    let email_param = TextParameter::builder()
        .metadata(
            ParameterMetadata::required("email", "Email", "Enter your email address").unwrap(),
        )
        .build();

    // Optional parameters (can be added via "Add Parameter" button)
    let age_param = NumberParameter::builder()
        .metadata(ParameterMetadata::new("age", "Age", "Enter your age").unwrap())
        .options(
            NumberParameterOptions::new()
                .with_display_mode(NumberDisplayMode::Text)
                .with_range(0.0, 150.0)
                .with_precision(0),
        )
        .build();

    let phone_param = TextParameter::builder()
        .metadata(ParameterMetadata::new("phone", "Phone Number", "Enter phone number").unwrap())
        .build();

    let website_param = TextParameter::builder()
        .metadata(ParameterMetadata::new("website", "Website", "Your personal website").unwrap())
        .build();

    let newsletter_param = CheckboxParameter::builder()
        .metadata(
            ParameterMetadata::new("newsletter", "Subscribe to Newsletter", "Get updates").unwrap(),
        )
        .build();

    let bio_param = TextParameter::builder()
        .metadata(ParameterMetadata::new("bio", "Biography", "Tell us about yourself").unwrap())
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
        .metadata(
            ParameterMetadata::new("email", "Email Address", "Enter recipient email").unwrap(),
        )
        .build();

    let sms_param = TextParameter::builder()
        .metadata(ParameterMetadata::new("phone", "Phone Number", "Enter phone number").unwrap())
        .build();

    let webhook_param = TextParameter::builder()
        .metadata(ParameterMetadata::new("url", "Webhook URL", "Enter webhook endpoint").unwrap())
        .build();

    ModeParameter::builder()
        .metadata(
            ParameterMetadata::new(
                "notification_mode",
                "Notification Method",
                "Choose how to send notifications",
            )
            .unwrap(),
        )
        .modes(vec![
            ModeItem::new("email", "Email", email_param)
                .with_description("Send notifications via email")
                .as_default(),
            ModeItem::new("sms", "SMS", sms_param).with_description("Send notifications via SMS"),
            ModeItem::new("webhook", "Webhook", webhook_param)
                .with_description("Send notifications to a webhook endpoint"),
        ])
        .build()
}
