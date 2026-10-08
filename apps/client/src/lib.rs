//! First-party HTTP workflow client. Document and session behavior are UI-independent.
#![forbid(unsafe_code)]
// Library code propagates typed errors; tests may still unwrap (see clippy.toml).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable
)]

#[cfg(feature = "ui")]
mod app;
#[cfg(feature = "ui")]
mod document;
#[cfg(feature = "ui")]
mod effects;
#[cfg(feature = "ui")]
mod session;
#[cfg(feature = "ui")]
mod theme;
#[cfg(feature = "ui")]
mod transport;
#[cfg(feature = "ui")]
mod views;
#[cfg(feature = "ui")]
mod widgets;
#[cfg(feature = "ui")]
mod workbench;

#[cfg(feature = "ui")]
pub use app::ClientApp;

#[cfg(all(feature = "ui", target_arch = "wasm32"))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub async fn start() -> Result<(), wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast;
    let canvas = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("nebula"))
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("Missing Nebula canvas"))?
        .dyn_into::<web_sys::HtmlCanvasElement>()?;
    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|cc| Ok(Box::new(ClientApp::new(cc)?))),
        )
        .await
}
