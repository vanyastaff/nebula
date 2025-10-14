use egui::Response;
use nebula_parameter::HiddenParameter;
use crate::{
    ParameterWidget, ParameterTheme, ParameterContext, ValidationState,
};

/// Widget for hidden parameters (not displayed in UI)
#[derive(Debug, Clone)]
pub struct HiddenWidget<'a> {
    parameter: HiddenParameter,
    context: ParameterContext<'a>,
}

impl<'a> HiddenWidget<'a> {
    pub fn new(parameter: HiddenParameter) -> Self {
        Self {
            parameter,
            context: ParameterContext::default(),
        }
    }

    pub fn with_context(mut self, context: ParameterContext) -> Self {
        self.context = context;
        self
    }
}

impl<'a> ParameterWidget for HiddenWidget<'a> {
    fn render(&mut self, ui: &mut egui::Ui) -> Response {
        // Hidden parameters don't render anything visible
        // They still need to return a response for consistency
        ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover())
    }

    fn render_with_theme(&mut self, ui: &mut egui::Ui, _theme: &ParameterTheme) -> Response {
        // Hidden parameters don't render anything visible
        // They still need to return a response for consistency
        ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover())
    }
}

/// Helper function to create a hidden widget
pub fn hidden_widget(parameter: HiddenParameter) -> HiddenWidget<'static> {
    HiddenWidget::new(parameter)
}
