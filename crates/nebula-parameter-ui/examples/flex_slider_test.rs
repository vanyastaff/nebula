//! Test flex slider layout

use eframe::NativeOptions;
use egui::{CentralPanel, Slider, Widget};
use egui_flex::{Flex, FlexAlign, FlexAlignContent, item};

fn main() -> eframe::Result {
    let mut value = 50.0f64;

    eframe::run_simple_native(
        "Flex Slider Test",
        NativeOptions::default(),
        move |ctx, _| {
            CentralPanel::default().show(ctx, |ui| {
                ui.heading("Flex Slider Test");
                ui.add_space(20.0);

                // Test 1: Simple horizontal with slider grow
                ui.label("Test 1: Slider with grow(1.0)");
                Flex::horizontal()
                    .w_full()
                    .align_items(FlexAlign::Center)
                    .align_content(FlexAlignContent::Stretch)
                    .show(ui, |flex| {
                        // Slider grows
                        flex.add_ui(item().grow(1.0).basis(100.0), |ui| {
                            ui.style_mut().spacing.slider_width = ui.available_width();
                            Slider::new(&mut value, 0.0..=100.0)
                                .show_value(false)
                                .ui(ui);
                        });

                        // Fixed label
                        flex.add_ui(item().basis(60.0), |ui| {
                            ui.label(format!("{:.1}%", value));
                        });
                    });

                ui.add_space(20.0);

                // Test 2: With wrap(false)
                ui.label("Test 2: With wrap(false)");
                Flex::horizontal()
                    .w_full()
                    .wrap(false)
                    .align_items(FlexAlign::Center)
                    .show(ui, |flex| {
                        flex.add_ui(item().grow(1.0).basis(100.0), |ui| {
                            ui.style_mut().spacing.slider_width = ui.available_width();
                            Slider::new(&mut value, 0.0..=100.0)
                                .show_value(false)
                                .ui(ui);
                        });

                        flex.add_ui(item().basis(60.0), |ui| {
                            ui.label(format!("{:.1}%", value));
                        });
                    });

                ui.add_space(20.0);

                // Test 3: Subtract from available_width
                ui.label("Test 3: available_width() - 50.0");
                Flex::horizontal()
                    .w_full()
                    .align_items(FlexAlign::Center)
                    .show(ui, |flex| {
                        flex.add_ui(item().grow(1.0).basis(150.0), |ui| {
                            ui.style_mut().spacing.slider_width = ui.available_width() - 50.0;
                            Slider::new(&mut value, 0.0..=100.0)
                                .show_value(false)
                                .ui(ui);
                        });

                        flex.add_ui(item().basis(60.0), |ui| {
                            ui.label(format!("{:.1}%", value));
                        });
                    });
            });
        },
    )
}
