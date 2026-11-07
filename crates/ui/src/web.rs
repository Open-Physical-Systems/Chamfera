//! Files in the browser (wasm32 only): the browser's file picker for Open and Insert Mesh,
//! downloads for Save and Export, and the current design kept in the browser's local
//! storage.
//!
//! The browser gives file contents, never paths, so these use the byte commands
//! (`design.open_bytes`, `body.import_stl_bytes`, `design.save_bytes`,
//! `design.export_bytes`), which carry the contents as base64. Picking is asynchronous: the
//! picker's callback puts the file in an inbox that the app empties on its next frame.
//!
//! Local storage is a per-browser convenience, not a file: the design is written there
//! shortly after each change and opened from there when the page loads. New clears it.

use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;

/// Where the design is kept in local storage.
const STORE_KEY: &str = "chamfera.design";
/// Its display name (the file it was opened from), if any.
const STORE_NAME_KEY: &str = "chamfera.design.name";
/// Files larger than this are not read in the browser (bytes): base64 and the parsed
/// mesh need several times the file's size in memory, and wasm32 has 4 GiB at most.
pub const MAX_PICK_BYTES: f64 = 256.0 * 1024.0 * 1024.0;
/// Seconds between a change and its autosave, so typing a value does not write each key.
const AUTOSAVE_DELAY: f64 = 0.75;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Design,
    Mesh,
    /// A DXF drawing for the active sketch.
    Dxf,
}

/// A file the user picked: its name, and its contents as base64 (or why it was refused).
pub struct Picked {
    pub kind: Kind,
    pub name: String,
    pub data: Result<String, String>,
}

#[derive(Default)]
pub struct WebFiles {
    inbox: Rc<RefCell<Vec<Picked>>>,
    /// The design as last written to (or read from) local storage.
    stored: Option<chamfera_doc::Document>,
    /// When the design was first seen to differ from `stored` (egui time, seconds).
    changed_at: Option<f64>,
    /// Storage failed once (full, or blocked): say so once, then stop trying.
    storage_failed: bool,
    /// The app's context, so a picked file can wake it up.
    ctx: Option<egui::Context>,
    /// The status last put on the page (`show_status`).
    status: String,
    /// Clickable things drawn this frame (`target`), by their text, and what the page
    /// last showed of them.
    targets: Vec<(String, egui::Rect)>,
    shown_targets: String,
}

impl WebFiles {
    /// Files picked since the last call.
    pub fn take_picked(&self) -> Vec<Picked> {
        self.inbox
            .try_borrow_mut()
            .map(|mut v| std::mem::take(&mut *v))
            .unwrap_or_default()
    }

    /// Remembers the app's context (each frame).
    pub fn set_context(&mut self, ctx: &egui::Context) {
        if self.ctx.is_none() {
            self.ctx = Some(ctx.clone());
        }
    }

    /// Opens the browser's file picker. `accept` is the input's accept list
    /// (".chamfera" or ".stl"). Picked files arrive in [`WebFiles::take_picked`].
    pub fn pick(&self, kind: Kind, accept: &str, multiple: bool) {
        let Some(ctx) = self.ctx.clone() else {
            return;
        };
        let Some(input) = document()
            .and_then(|d| d.create_element("input").ok())
            .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        else {
            return;
        };
        input.set_type("file");
        input.set_accept(accept);
        input.set_multiple(multiple);
        let inbox = self.inbox.clone();
        let target = input.clone();
        let on_change = Closure::once_into_js(move || {
            let Some(files) = target.files() else {
                return;
            };
            for i in 0..files.length() {
                if let Some(file) = files.get(i) {
                    read_file(file, kind, inbox.clone(), ctx.clone());
                }
            }
        });
        input.set_onchange(Some(on_change.unchecked_ref()));
        input.click();
    }

    /// The design to restore at start, if this browser has one: (JSON text, name).
    pub fn stored_design() -> Option<(String, Option<String>)> {
        let storage = local_storage()?;
        let text = storage.get_item(STORE_KEY).ok().flatten()?;
        let name = storage.get_item(STORE_NAME_KEY).ok().flatten();
        Some((text, name))
    }

    /// Records the design just restored, so it is not written straight back.
    pub fn restored(&mut self, doc: &chamfera_doc::Document) {
        self.stored = Some(doc.clone());
        self.changed_at = None;
    }

    /// Forgets the stored design (File > New).
    pub fn clear(&mut self) {
        if let Some(storage) = local_storage() {
            let _ = storage.remove_item(STORE_KEY);
            let _ = storage.remove_item(STORE_NAME_KEY);
        }
        self.stored = Some(chamfera_doc::Document::default());
        self.changed_at = None;
    }

    /// Writes the design to local storage a moment after it changes. An empty design is
    /// not kept. Returns a message to show when storage fails the first time.
    pub fn autosave(
        &mut self,
        ctx: &egui::Context,
        doc: &chamfera_doc::Document,
        name: Option<&str>,
    ) -> Option<String> {
        if self.storage_failed || self.stored.as_ref() == Some(doc) {
            self.changed_at = None;
            return None;
        }
        let now = ctx.input(|i| i.time);
        let since = *self.changed_at.get_or_insert(now);
        if now - since < AUTOSAVE_DELAY {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(AUTOSAVE_DELAY));
            return None;
        }
        self.changed_at = None;
        self.stored = Some(doc.clone());
        let storage = local_storage()?;
        if *doc == chamfera_doc::Document::default() {
            let _ = storage.remove_item(STORE_KEY);
            let _ = storage.remove_item(STORE_NAME_KEY);
            return None;
        }
        let Ok(text) = doc.to_json() else {
            return None;
        };
        let written = storage.set_item(STORE_KEY, &text).is_ok()
            && match name {
                Some(n) => storage.set_item(STORE_NAME_KEY, n).is_ok(),
                None => storage.remove_item(STORE_NAME_KEY).is_ok(),
            };
        if written {
            None
        } else {
            self.storage_failed = true;
            Some(
                "The browser did not let Chamfera keep this design (storage full or blocked)"
                    .into(),
            )
        }
    }
}

fn document() -> Option<web_sys::Document> {
    web_sys::window().and_then(|w| w.document())
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

/// Reads one picked file as a data URL (the browser does the base64) into the inbox.
fn read_file(file: web_sys::File, kind: Kind, inbox: Rc<RefCell<Vec<Picked>>>, ctx: egui::Context) {
    let name: String = file.name().chars().take(256).collect();
    let deliver = move |picked: Picked| {
        if let Ok(mut v) = inbox.try_borrow_mut() {
            v.push(picked);
        }
        ctx.request_repaint();
    };
    if file.size() > MAX_PICK_BYTES {
        deliver(Picked {
            kind,
            data: Err(format!(
                "{name} is {:.0} MB; the browser app reads files up to {:.0} MB",
                file.size() / 1e6,
                MAX_PICK_BYTES / 1e6
            )),
            name,
        });
        return;
    }
    let Ok(reader) = web_sys::FileReader::new() else {
        return;
    };
    let source = reader.clone();
    let file_name = name.clone();
    let on_load = Closure::once_into_js(move || {
        let data = source
            .result()
            .ok()
            .and_then(|v| v.as_string())
            .and_then(|url| url.split_once(',').map(|(_, b64)| b64.to_owned()))
            .ok_or_else(|| format!("{file_name} could not be read"));
        deliver(Picked {
            kind,
            name: file_name,
            data,
        });
    });
    reader.set_onloadend(Some(on_load.unchecked_ref()));
    if reader.read_as_data_url(&file).is_err() {
        reader.set_onloadend(None);
    }
}

/// Offers `data` (base64) to the user as a download named `name`.
pub fn download(name: &str, mime: &str, data: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or("no browser window")?;
    let binary = window
        .atob(data)
        .map_err(|_| "the file's contents are not base64".to_owned())?;
    let bytes: Vec<u8> = binary
        .chars()
        .map(|c| u8::try_from(c).unwrap_or(0))
        .collect();
    let array = js_sys::Uint8Array::from(bytes.as_slice());
    let parts = js_sys::Array::of1(&array);
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .map_err(|_| "the browser could not make the file".to_owned())?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|_| "the browser could not make the file".to_owned())?;
    let anchor = window
        .document()
        .and_then(|d| d.create_element("a").ok())
        .and_then(|e| e.dyn_into::<web_sys::HtmlAnchorElement>().ok())
        .ok_or("the browser could not start the download")?;
    anchor.set_href(&url);
    anchor.set_download(name);
    anchor.click();
    // Let the download start before the object URL goes.
    let revoke = Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    let _ = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 30_000);
    Ok(())
}

/// Base64 of `bytes`, by the browser.
pub fn to_base64(bytes: &[u8]) -> Option<String> {
    let binary: String = bytes.iter().map(|&b| char::from(b)).collect();
    web_sys::window()?.btoa(&binary).ok()
}

/// The current time as `YYYY-MM-DDThh:mm:ss` (UTC), for STEP headers.
pub fn now_iso8601() -> String {
    let iso = String::from(js_sys::Date::new_0().to_iso_string());
    iso.chars().take(19).collect()
}

impl WebFiles {
    /// Puts the status bar's message and the body count on `<body>` as `data-message`
    /// and `data-bodies`, so tests in a browser (`tools/web-smoke.mjs`) can read them.
    pub fn show_status(&mut self, message: &str, bodies: usize) {
        let status = format!("{bodies}\n{message}");
        if status == self.status {
            return;
        }
        if let Some(body) = document().and_then(|d| d.body()) {
            let _ = body.dataset().set("message", message);
            let _ = body.dataset().set("bodies", &bodies.to_string());
        }
        self.status = status;
    }

    /// Records a clickable thing (a menu or menu item) drawn this frame, by its text.
    pub fn target(&mut self, text: &str, rect: egui::Rect) {
        self.targets.push((text.to_owned(), rect));
    }

    /// Puts the targets of this frame on `<body>` as `data-targets`, a JSON object of
    /// text to the centre in CSS pixels, so browser tests click menu items by their text
    /// instead of by coordinates. Call once at the end of each frame.
    pub fn show_targets(&mut self, zoom: f32) {
        let targets: serde_json::Map<String, serde_json::Value> = self
            .targets
            .drain(..)
            .map(|(text, rect)| {
                let c = rect.center();
                (text, serde_json::json!([c.x * zoom, c.y * zoom]))
            })
            .collect();
        let json = serde_json::Value::Object(targets).to_string();
        if json != self.shown_targets {
            if let Some(body) = document().and_then(|d| d.body()) {
                let _ = body.dataset().set("targets", &json);
            }
            self.shown_targets = json;
        }
    }
}

/// Sets the page title (the browser's tab).
pub fn set_title(title: &str) {
    if let Some(d) = document() {
        d.set_title(title);
    }
}
