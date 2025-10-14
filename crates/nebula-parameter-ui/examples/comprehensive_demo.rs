//! Comprehensive demo showcasing all nebula-parameter-ui widgets

use eframe::egui;
use nebula_parameter::{
    TextParameter, NumberParameter, CheckboxParameter, SelectParameter,
    TextareaParameter, DateParameter, TimeParameter, DateTimeParameter,
    SecretParameter, ColorParameter, RadioParameter, MultiSelectParameter,
    CodeParameter, FileParameter, ParameterMetadata, SelectOption,
};
use nebula_parameter_ui::{
    ParameterWidget, ParameterTheme,
    TextWidget, NumberWidget, CheckboxWidget, SelectWidget,
    TextareaWidget, DateWidget, TimeWidget, DateTimeWidget,
    SecretWidget, ColorWidget, RadioWidget, MultiSelectWidget,
    CodeWidget, FileWidget, SliderWidget,
    // LayoutConfig, adaptive_container,  // Temporarily disabled
    // FlexConfig, flex_container, FlexDirection, AlignItems,  // Temporarily disabled
    // GridConfig, grid_container,  // Temporarily disabled
};
use nebula_value::Boolean;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 800.0])
            .with_title("Nebula Parameter UI - Comprehensive Demo"),
        ..Default::default()
    };

    eframe::run_native(
        "Nebula Parameter UI Demo",
        options,
        Box::new(|_cc| Ok(Box::new(DemoApp::new()))),
    )
}

struct DemoApp {
    // Basic widgets
    text_widget: TextWidget,
    textarea_widget: TextareaWidget,
    number_widget: NumberWidget,
    slider_widget: SliderWidget,
    checkbox_widget: CheckboxWidget,
    secret_widget: SecretWidget,
    
    // Selection widgets
    select_widget: SelectWidget,
    radio_widget: RadioWidget,
    multi_select_widget: MultiSelectWidget,
    
    // Date/Time widgets
    date_widget: DateWidget,
    time_widget: TimeWidget,
    datetime_widget: DateTimeWidget,
    
    // Specialized widgets
    color_widget: ColorWidget,
    code_widget: CodeWidget,
    file_widget: FileWidget,
    
    // Theme
    theme: ParameterTheme,
    use_light_theme: bool,
}

impl DemoApp {
    fn new() -> Self {
        Self {
            // Basic widgets
            text_widget: TextWidget::new(create_text_param()),
            textarea_widget: TextareaWidget::new(create_textarea_param()),
            number_widget: NumberWidget::new(create_number_param()),
            slider_widget: SliderWidget::new(create_slider_param()),
            checkbox_widget: CheckboxWidget::new(create_checkbox_param()),
            secret_widget: SecretWidget::new(create_secret_param()),
            
            // Selection widgets
            select_widget: SelectWidget::new(create_select_param()),
            radio_widget: RadioWidget::new(create_radio_param()),
            multi_select_widget: MultiSelectWidget::new(create_multi_select_param()),
            
            // Date/Time widgets
            date_widget: DateWidget::new(create_date_param()),
            time_widget: TimeWidget::new(create_time_param()),
            datetime_widget: DateTimeWidget::new(create_datetime_param()),
            
            // Specialized widgets
            color_widget: ColorWidget::new(create_color_param()),
            code_widget: CodeWidget::new(create_code_param()),
            file_widget: FileWidget::new(create_file_param()),
            
            // Theme
            theme: ParameterTheme::dark(),
            use_light_theme: false,
        }
    }
}

impl eframe::App for DemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Top panel for theme switcher
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("🚀 Nebula Parameter UI - Comprehensive Demo");
                ui.separator();
                
                if ui.selectable_label(!self.use_light_theme, "🌙 Dark").clicked() {
                    self.use_light_theme = false;
                    self.theme = ParameterTheme::dark();
                }
                if ui.selectable_label(self.use_light_theme, "☀ Light").clicked() {
                    self.use_light_theme = true;
                    self.theme = ParameterTheme::light();
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(10.0);
                
                // Basic Input Widgets - Flex Layout with proper alignment
                ui.heading("📝 Basic Input Widgets");
                ui.separator();
                
                // First row: Text, Number - Two columns with proper width calculation
                ui.horizontal(|ui| {
                    let available_width = ui.available_width();
                    let half_width = (available_width - 16.0) / 2.0; // 16px gap between widgets
                    
                    ui.allocate_ui(egui::Vec2::new(half_width, 0.0), |ui| {
                        ui.group(|ui| {
                            self.text_widget.render_with_theme(ui, &self.theme);
                        });
                    });
                    
                    ui.allocate_ui(egui::Vec2::new(half_width, 0.0), |ui| {
                        ui.group(|ui| {
                            self.number_widget.render_with_theme(ui, &self.theme);
                        });
                    });
                });
                
                // Slider takes full width
                ui.group(|ui| {
                    self.slider_widget.render_with_theme(ui, &self.theme);
                });
                
                // Second row: Checkbox, Secret - Two columns with proper width calculation
                ui.horizontal(|ui| {
                    let available_width = ui.available_width();
                    let half_width = (available_width - 16.0) / 2.0; // 16px gap between widgets
                    
                    ui.allocate_ui(egui::Vec2::new(half_width, 0.0), |ui| {
                        ui.group(|ui| {
                            self.checkbox_widget.render_with_theme(ui, &self.theme);
                        });
                    });
                    
                    ui.allocate_ui(egui::Vec2::new(half_width, 0.0), |ui| {
                        ui.group(|ui| {
                            self.secret_widget.render_with_theme(ui, &self.theme);
                        });
                    });
                });
                
                // Textarea takes full width
                ui.group(|ui| {
                    self.textarea_widget.render_with_theme(ui, &self.theme);
                });
                
                ui.add_space(20.0);
                
                // Selection Widgets - Even Grid Layout
                ui.heading("🎯 Selection Widgets");
                ui.separator();
                
                // Selection widgets - Two columns with proper width calculation
                ui.horizontal(|ui| {
                    let available_width = ui.available_width();
                    let half_width = (available_width - 16.0) / 2.0; // 16px gap between widgets
                    
                    ui.allocate_ui(egui::Vec2::new(half_width, 0.0), |ui| {
                        ui.group(|ui| {
                            self.select_widget.render_with_theme(ui, &self.theme);
                        });
                    });
                    
                    ui.allocate_ui(egui::Vec2::new(half_width, 0.0), |ui| {
                        ui.group(|ui| {
                            self.radio_widget.render_with_theme(ui, &self.theme);
                        });
                    });
                });
                
                // Multi-select takes more space
                ui.group(|ui| {
                    self.multi_select_widget.render_with_theme(ui, &self.theme);
                });
                
                ui.add_space(20.0);
                
                // Date/Time Widgets - Even Grid Layout
                ui.heading("📅 Date & Time Widgets");
                ui.separator();
                
                // Date/Time widgets - Two columns with proper width calculation
                ui.horizontal(|ui| {
                    let available_width = ui.available_width();
                    let half_width = (available_width - 16.0) / 2.0; // 16px gap between widgets
                    
                    ui.allocate_ui(egui::Vec2::new(half_width, 0.0), |ui| {
                        ui.group(|ui| {
                            self.date_widget.render_with_theme(ui, &self.theme);
                        });
                    });
                    
                    ui.allocate_ui(egui::Vec2::new(half_width, 0.0), |ui| {
                        ui.group(|ui| {
                            self.time_widget.render_with_theme(ui, &self.theme);
                        });
                    });
                });
                
                // DateTime takes full width
                ui.group(|ui| {
                    self.datetime_widget.render_with_theme(ui, &self.theme);
                });
                
                ui.add_space(20.0);
                
                // Specialized Widgets - Even Grid Layout
                ui.heading("🎨 Specialized Widgets");
                ui.separator();
                
                // Specialized widgets - Two columns with proper width calculation
                ui.horizontal(|ui| {
                    let available_width = ui.available_width();
                    let half_width = (available_width - 16.0) / 2.0; // 16px gap between widgets
                    
                    ui.allocate_ui(egui::Vec2::new(half_width, 0.0), |ui| {
                        ui.group(|ui| {
                            self.color_widget.render_with_theme(ui, &self.theme);
                        });
                    });
                    
                    ui.allocate_ui(egui::Vec2::new(half_width, 0.0), |ui| {
                        ui.group(|ui| {
                            self.file_widget.render_with_theme(ui, &self.theme);
                        });
                    });
                });
                
                // Code widget takes full width
                ui.group(|ui| {
                    self.code_widget.render_with_theme(ui, &self.theme);
                });
                
                ui.add_space(20.0);
            });
        });
    }
}

// Parameter creation functions

fn create_text_param() -> TextParameter {
    TextParameter {
        metadata: ParameterMetadata::builder()
            .key("username")
            .name("Username")
            .description("Enter your username or email address")
            .required(true)
            .placeholder("user@example.com".to_string())
            .hint("This field is required for authentication".to_string())
            .build()
            .expect("valid metadata"),
        value: None,
        default: Some(nebula_value::Text::from("demo_user")),
        options: None,
        display: None,
        validation: None,
    }
}

fn create_textarea_param() -> TextareaParameter {
    TextareaParameter {
        metadata: ParameterMetadata::builder()
            .key("description")
            .name("Description")
            .description("Provide a detailed description")
            .placeholder("Enter multiple lines of text...".to_string())
            .hint("Markdown formatting is supported".to_string())
            .build()
            .expect("valid metadata"),
        value: None,
        default: None,
        options: None,
        display: None,
        validation: None,
    }
}

fn create_number_param() -> NumberParameter {
    NumberParameter {
        metadata: ParameterMetadata::builder()
            .key("port")
            .name("Port Number")
            .description("Specify the port number for the connection")
            .required(true)
            .placeholder("8080".to_string())
            .hint("Use ports 1024-65535 for non-privileged services".to_string())
            .build()
            .expect("valid metadata"),
        value: Some(8080.0),
        default: Some(8080.0),
        options: Some(nebula_parameter::NumberParameterOptions {
            min: Some(1024.0),
            max: Some(65535.0),
            step: Some(1.0),
            precision: Some(0),
        }),
        display: None,
        validation: None,
    }
}

fn create_slider_param() -> NumberParameter {
    NumberParameter {
        metadata: ParameterMetadata::builder()
            .key("volume")
            .name("Volume Level")
            .description("Adjust the volume level")
            .build()
            .expect("valid metadata"),
        value: Some(50.0),
        default: Some(50.0),
        options: Some(nebula_parameter::NumberParameterOptions {
            min: Some(0.0),
            max: Some(100.0),
            step: Some(5.0),
            precision: Some(0),
        }),
        display: None,
        validation: None,
    }
}

fn create_checkbox_param() -> CheckboxParameter {
    CheckboxParameter {
        metadata: ParameterMetadata::builder()
            .key("agree_terms")
            .name("I agree to the terms and conditions")
            .description("Please read and accept our terms")
            .required(true)
            .build()
            .expect("valid metadata"),
        value: Some(Boolean::new(false)),
        default: Some(Boolean::new(false)),
        options: None,
        display: None,
        validation: None,
    }
}

fn create_secret_param() -> SecretParameter {
    SecretParameter {
        metadata: ParameterMetadata::builder()
            .key("api_key")
            .name("API Key")
            .description("Enter your secret API key")
            .required(true)
            .placeholder("sk-...".to_string())
            .hint("Keep this key secure and never share it".to_string())
            .build()
            .expect("valid metadata"),
        value: None,
        default: None,
        options: None,
        display: None,
        validation: None,
    }
}

fn create_select_param() -> SelectParameter {
    SelectParameter {
        metadata: ParameterMetadata::builder()
            .key("environment")
            .name("Environment")
            .description("Choose the deployment environment")
            .required(true)
            .hint("Different environments have different configurations".to_string())
            .build()
            .expect("valid metadata"),
        value: None,
        default: None,
        options: vec![
            SelectOption::simple("dev", "Development"),
            SelectOption::simple("staging", "Staging"),
            SelectOption::simple("prod", "Production"),
        ],
        select_options: None,
        display: None,
        validation: None,
    }
}

fn create_radio_param() -> RadioParameter {
    RadioParameter {
        metadata: ParameterMetadata::builder()
            .key("priority")
            .name("Priority Level")
            .description("Select the priority level for this task")
            .required(true)
            .build()
            .expect("valid metadata"),
        value: None,
        default: None,
        options: vec![
            SelectOption::with_description("low", "Low", "low", std::borrow::Cow::Borrowed("For non-urgent tasks")),
            SelectOption::with_description("medium", "Medium", "medium", std::borrow::Cow::Borrowed("Standard priority")),
            SelectOption::with_description("high", "High", "high", std::borrow::Cow::Borrowed("Urgent tasks")),
        ],
        radio_options: None,
        display: None,
        validation: None,
    }
}

fn create_multi_select_param() -> MultiSelectParameter {
    MultiSelectParameter {
        metadata: ParameterMetadata::builder()
            .key("features")
            .name("Features")
            .description("Select the features to enable")
            .hint("You can select multiple features".to_string())
            .build()
            .expect("valid metadata"),
        value: None,
        default: None,
        options: vec![
            SelectOption::simple("auth", "Authentication"),
            SelectOption::simple("logging", "Logging"),
            SelectOption::simple("metrics", "Metrics"),
            SelectOption::simple("caching", "Caching"),
        ],
        multi_select_options: None,
        display: None,
        validation: None,
    }
}

fn create_date_param() -> DateParameter {
    DateParameter {
        metadata: ParameterMetadata::builder()
            .key("start_date")
            .name("Start Date")
            .description("Select the start date")
            .placeholder("YYYY-MM-DD".to_string())
            .build()
            .expect("valid metadata"),
        value: None,
        default: None,
        options: None,
        display: None,
        validation: None,
    }
}

fn create_time_param() -> TimeParameter {
    TimeParameter {
        metadata: ParameterMetadata::builder()
            .key("start_time")
            .name("Start Time")
            .description("Select the start time")
            .placeholder("HH:MM:SS".to_string())
            .build()
            .expect("valid metadata"),
        value: None,
        default: None,
        options: None,
        display: None,
        validation: None,
    }
}

fn create_datetime_param() -> DateTimeParameter {
    DateTimeParameter {
        metadata: ParameterMetadata::builder()
            .key("scheduled_at")
            .name("Scheduled DateTime")
            .description("When should this be executed?")
            .placeholder("YYYY-MM-DD HH:MM:SS".to_string())
            .build()
            .expect("valid metadata"),
        value: None,
        default: None,
        options: None,
        display: None,
        validation: None,
    }
}

fn create_color_param() -> ColorParameter {
    ColorParameter {
        metadata: ParameterMetadata::builder()
            .key("theme_color")
            .name("Theme Color")
            .description("Choose your preferred theme color")
            .build()
            .expect("valid metadata"),
        value: Some(nebula_value::Text::from("#3498db")),
        default: Some(nebula_value::Text::from("#3498db")),
        options: None,
        display: None,
        validation: None,
    }
}

fn create_code_param() -> CodeParameter {
    CodeParameter {
        metadata: ParameterMetadata::builder()
            .key("custom_script")
            .name("Custom Script")
            .description("Write your custom JavaScript code")
            .placeholder("console.log('Hello, World!');".to_string())
            .hint("This code will be executed in the runtime".to_string())
            .build()
            .expect("valid metadata"),
        value: Some(nebula_value::Text::from("// Your code here\nconsole.log('Hello!');")),
        default: None,
        options: Some(nebula_parameter::CodeParameterOptions {
            language: Some(nebula_parameter::CodeLanguage::JavaScript),
            readonly: false,
        }),
        display: None,
        validation: None,
    }
}

fn create_file_param() -> FileParameter {
    FileParameter {
        metadata: ParameterMetadata::builder()
            .key("config_file")
            .name("Configuration File")
            .description("Select a configuration file to upload")
            .hint("Supported formats: JSON, YAML, TOML".to_string())
            .build()
            .expect("valid metadata"),
        value: None,
        default: None,
        options: None,
        display: None,
        validation: None,
    }
}

