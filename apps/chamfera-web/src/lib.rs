//! Chamfera in the browser. Build with `trunk build --release` in this folder (see index.html).
//!
//! The same UI crate as the desktop app runs here; the sample part is shown at start. Mesh
//! import from the browser's file picker comes later (it needs file bytes, not paths).

#[cfg(target_arch = "wasm32")]
mod web {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::wasm_bindgen;

    #[wasm_bindgen(start)]
    pub fn start() {
        wasm_bindgen_futures::spawn_local(async {
            let Some(canvas) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id("chamfera"))
                .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            else {
                return;
            };
            let options = chamfera_ui::Options {
                sample: true,
                ..Default::default()
            };
            let _ = eframe::WebRunner::new()
                .start(
                    canvas,
                    eframe::WebOptions::default(),
                    Box::new(move |cc| Ok(Box::new(chamfera_ui::ChamferaApp::new(cc, options)))),
                )
                .await;
        });
    }
}
