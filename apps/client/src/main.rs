#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Nebula",
        options,
        Box::new(|cc| Ok(Box::new(nebula_client_app::ClientApp::new(cc)?))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}
