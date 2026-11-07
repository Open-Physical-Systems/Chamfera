//! Chamfera in the browser. Build with `trunk build --release` in this folder (see
//! index.html), or `cargo xtask wasm` to only check that it compiles.
//!
//! The same UI crate as the desktop app runs here. Files go through the browser: Open and
//! Insert Mesh use its file picker, Save and Export download the file, and the current
//! design is kept in the browser's local storage (`chamfera_ui::web`).
//!
//! Rendering uses WebGPU where the browser has it and WebGL 2 otherwise; `?renderer=webgl`
//! in the address forces WebGL 2 (and `?renderer=webgpu` WebGPU). Once the app runs, `<body>` gets
//! `data-chamfera="running"` and `data-backend` (or `data-chamfera="failed"` and
//! `data-error`), which `tools/web-smoke.mjs` waits for.

#[cfg(target_arch = "wasm32")]
mod web {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::wasm_bindgen;

    /// Marks the page with the app's state, for the smoke test and for bug reports.
    fn mark(key: &str, value: &str) {
        if let Some(body) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.body())
        {
            let _ = body.dataset().set(key, value);
        }
    }

    /// `?renderer=webgl` or `?renderer=webgpu` in the page address: only that backend.
    fn forced_backends() -> Option<wgpu::Backends> {
        let query = web_sys::window().and_then(|w| w.location().search().ok())?;
        if query.contains("renderer=webgl") {
            Some(wgpu::Backends::GL)
        } else if query.contains("renderer=webgpu") {
            Some(wgpu::Backends::BROWSER_WEBGPU)
        } else {
            None
        }
    }

    fn options() -> eframe::WebOptions {
        let mut options = eframe::WebOptions::default();
        if let Some(backends) = forced_backends()
            && let egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup
        {
            setup.instance_descriptor.backends = backends;
        }
        options
    }

    #[wasm_bindgen(start)]
    pub fn start() {
        // A panic shows in the browser console with its message, not as "unreachable".
        console_error_panic_hook::set_once();
        let _ = eframe::WebLogger::init(log::LevelFilter::Warn);
        wasm_bindgen_futures::spawn_local(async {
            let Some(canvas) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id("chamfera"))
                .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            else {
                mark("chamfera", "failed");
                mark("error", "no canvas#chamfera in the page");
                return;
            };
            let app_options = chamfera_ui::Options {
                sample: true,
                ..Default::default()
            };
            let started = eframe::WebRunner::new()
                .start(
                    canvas,
                    options(),
                    Box::new(move |cc| {
                        let backend = cc.wgpu_render_state.as_ref().map_or_else(
                            || "none".to_owned(),
                            |rs| format!("{:?}", rs.adapter.get_info().backend),
                        );
                        mark("backend", &backend);
                        Ok(Box::new(chamfera_ui::ChamferaApp::new(cc, app_options)))
                    }),
                )
                .await;
            match started {
                Ok(()) => {
                    mark("chamfera", "running");
                    hide_loading();
                }
                Err(e) => {
                    let text = e.as_string().unwrap_or_else(|| format!("{e:?}"));
                    log::error!("Chamfera could not start: {text}");
                    mark("chamfera", "failed");
                    mark("error", &text);
                    show_failure(&text);
                }
            }
        });
    }

    fn element(id: &str) -> Option<web_sys::HtmlElement> {
        web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(id))
            .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    }

    fn hide_loading() {
        if let Some(e) = element("loading") {
            e.remove();
        }
    }

    fn show_failure(text: &str) {
        if let Some(e) = element("loading") {
            e.set_inner_text(&format!(
                "Chamfera could not start in this browser: {text}\n\nIt needs WebGPU or WebGL 2."
            ));
        }
    }
}
