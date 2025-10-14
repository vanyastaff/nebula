use egui::{Response, Ui, Color32, RichText};
use nebula_parameter::{NoticeParameter, NoticeType};
use crate::{
    ParameterWidget, ParameterTheme, ParameterContext, ValidationState,
};

/// Widget for displaying notice/information messages
#[derive(Debug, Clone)]
pub struct NoticeWidget<'a> {
    parameter: NoticeParameter,
    context: ParameterContext<'a>,
}

impl<'a> NoticeWidget<'a> {
    pub fn new(parameter: NoticeParameter) -> Self {
        Self {
            parameter,
            context: ParameterContext::default(),
        }
    }

    pub fn with_context(mut self, context: ParameterContext) -> Self {
        self.context = context;
        self
    }

    fn get_notice_color(&self, theme: &ParameterTheme) -> Color32 {
        match self.parameter.options.as_ref()
            .and_then(|opts| opts.notice_type.as_ref())
            .unwrap_or(&NoticeType::Info)
        {
            NoticeType::Info => theme.colors.info,
            NoticeType::Warning => theme.colors.warning,
            NoticeType::Error => theme.colors.error,
            NoticeType::Success => theme.colors.success,
        }
    }

    fn get_notice_icon(&self) -> &'static str {
        match self.parameter.options.as_ref()
            .and_then(|opts| opts.notice_type.as_ref())
            .unwrap_or(&NoticeType::Info)
        {
            NoticeType::Info => "ℹ️",
            NoticeType::Warning => "⚠️",
            NoticeType::Error => "❌",
            NoticeType::Success => "✅",
        }
    }
}

impl<'a> ParameterWidget for NoticeWidget<'a> {
    fn render(&mut self, ui: &mut Ui) -> Response {
        self.render_with_theme(ui, &ParameterTheme::default())
    }

    fn render_with_theme(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> Response {
        let notice_color = self.get_notice_color(theme);
        let icon = self.get_notice_icon();

        // Create a colored frame for the notice
        let frame = egui::Frame::none()
            .fill(notice_color.gamma_multiply(0.1))
            .stroke(egui::Stroke::new(1.0, notice_color.gamma_multiply(0.5)))
            .inner_margin(egui::Margin::symmetric(12.0, 8.0));

        let response = frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                // Icon
                ui.label(RichText::new(icon).size(16.0));

                // Content
                ui.vertical(|ui| {
                    // Title (parameter name)
                    if !self.parameter.metadata.name.is_empty() {
                        ui.label(
                            RichText::new(&self.parameter.metadata.name)
                                .color(notice_color)
                                .strong()
                        );
                    }

                    // Content text
                    ui.label(
                        RichText::new(&self.parameter.content)
                            .color(ui.style().visuals.text_color())
                    );

                    // Description if available
                    if let Some(description) = &self.parameter.metadata.description {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(description)
                                .color(ui.style().visuals.weak_text_color())
                                .small()
                        );
                    }
                });

                // Dismiss button if dismissible
                if self.parameter.options.as_ref()
                    .map(|opts| opts.dismissible)
                    .unwrap_or(false)
                {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").clicked() {
                            // Note: In a real implementation, this would need to communicate
                            // back to the parent that this notice should be hidden
                            // For now, we'll just log it
                            println!("Notice dismissed: {}", self.parameter.metadata.key);
                        }
                    });
                }
            });
        });

        response.response
    }
}

/// Helper function to create a notice widget
pub fn notice_widget(parameter: NoticeParameter) -> NoticeWidget<'static> {
    NoticeWidget::new(parameter)
}
