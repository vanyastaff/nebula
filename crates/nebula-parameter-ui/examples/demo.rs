//! Demo application showcasing all parameter widgets

use eframe::egui;
use nebula_parameter::{
    CheckboxParameter, DateParameter, NumberParameter, ParameterMetadata, SelectOption,
    SelectParameter, TextParameter, TextareaParameter, TimeParameter,
};
use nebula_parameter_ui::{
    CheckboxWidget, DateWidget, NumberWidget, ParameterWidget, SelectWidget, TextWidget,
    TextareaWidget, TimeWidget,
};
use nebula_value::Boolean;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_title("Nebula Parameter UI Demo"),
        ..Default::default()
    };

    eframe::run_native(
        "Nebula Parameter UI Demo",
        options,
        Box::new(|_cc| Ok(Box::new(DemoApp::new()))),
    )
}

struct DemoApp {
    text_widget: TextWidget,
    number_widget: NumberWidget,
    checkbox_widget: CheckboxWidget,
    select_widget: SelectWidget,
    textarea_widget: TextareaWidget,
    date_widget: DateWidget,
    time_widget: TimeWidget,
}

impl DemoApp {
    fn new() -> Self {
        // Create text parameter
        let text_param = TextParameter {
            metadata: ParameterMetadata::builder()
                .key("text_input")
                .name("Text Input")
                .description("Enter your username or email address")
                .required(true)
                .placeholder("username@example.com".to_string())
                .hint("This field is required for authentication".to_string())
                .build()
                .expect("valid metadata"),
            default: Some(nebula_value::Text::from("Default text")),
            options: None,
            display: None,
            validation: None,
        };

        // Create number parameter
        let number_param = NumberParameter {
            metadata: ParameterMetadata::builder()
                .key("number_input")
                .name("Port Number")
                .description("Specify the port number for the connection")
                .required(true)
                .placeholder("8080".to_string())
                .hint("Use ports 1024-65535 for non-privileged services".to_string())
                .build()
                .expect("valid metadata"),
            default: Some(8080.0),
            options: None,
            display: None,
            validation: None,
        };

        // Create checkbox parameter
        let checkbox_param = CheckboxParameter {
            metadata: ParameterMetadata::builder()
                .key("checkbox_input")
                .name("Checkbox")
                .description("Example checkbox parameter")
                .build()
                .expect("valid metadata"),
            default: Some(Boolean::new(false)),
            options: None,
            display: None,
            validation: None,
        };

        // Create select parameter
        let select_param = SelectParameter {
            metadata: ParameterMetadata::builder()
                .key("select_input")
                .name("Environment")
                .description("Choose the deployment environment")
                .required(true)
                .hint("Different environments have different configurations".to_string())
                .build()
                .expect("valid metadata"),
            default: None,
            options: vec![
                SelectOption::simple("dev", "Development"),
                SelectOption::simple("staging", "Staging"),
                SelectOption::simple("prod", "Production"),
            ],
            select_options: None,
            display: None,
            validation: None,
        };

        // Create textarea parameter
        let textarea_param = TextareaParameter {
            metadata: ParameterMetadata::builder()
                .key("textarea_input")
                .name("Description")
                .description("Provide a detailed description of your configuration")
                .placeholder("Enter multiple lines of text...".to_string())
                .hint("Markdown formatting is supported".to_string())
                .build()
                .expect("valid metadata"),
            default: None,
            options: None,
            display: None,
            validation: None,
        };

        // Create date parameter
        let date_param = DateParameter {
            metadata: ParameterMetadata::builder()
                .key("date_input")
                .name("Date")
                .description("Example date parameter")
                .build()
                .expect("valid metadata"),
            default: None,
            options: None,
            display: None,
            validation: None,
        };

        // Create time parameter
        let time_param = TimeParameter {
            metadata: ParameterMetadata::builder()
                .key("time_input")
                .name("Time")
                .description("Example time parameter")
                .build()
                .expect("valid metadata"),
            default: None,
            options: None,
            display: None,
            validation: None,
        };

        Self {
            text_widget: TextWidget::new(text_param),
            number_widget: NumberWidget::new(number_param),
            checkbox_widget: CheckboxWidget::new(checkbox_param),
            select_widget: SelectWidget::new(select_param),
            textarea_widget: TextareaWidget::new(textarea_param),
            date_widget: DateWidget::new(date_param),
            time_widget: TimeWidget::new(time_param),
        }
    }
}

impl eframe::App for DemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Nebula Parameter UI Demo");
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.group(|ui| {
                    self.text_widget.render(ui);
                });

                ui.add_space(10.0);

                ui.group(|ui| {
                    self.number_widget.render(ui);
                });

                ui.add_space(10.0);

                ui.group(|ui| {
                    self.checkbox_widget.render(ui);
                });

                ui.add_space(10.0);

                ui.group(|ui| {
                    self.select_widget.render(ui);
                });

                ui.add_space(10.0);

                ui.group(|ui| {
                    self.textarea_widget.render(ui);
                });

                ui.add_space(10.0);

                ui.group(|ui| {
                    self.date_widget.render(ui);
                });

                ui.add_space(10.0);

                ui.group(|ui| {
                    self.time_widget.render(ui);
                });
            });
        });
    }
}
