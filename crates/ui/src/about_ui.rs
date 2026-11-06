//! Help > About Chamfera: the version, Chamfera's licence, and the third-party software
//! inside the app (`ATTRIBUTION.md`, built in, and the licence texts of every bundled crate
//! in `THIRD-PARTY-LICENSES.txt`, which the packages ship next to the app; see
//! `cargo xtask notices`).

use crate::ChamferaApp;
use crate::icons::Icon;
use crate::widgets::{self, Look, T_BODY, T_SMALL, T_TITLE};

/// The attribution file, built into the app.
const ATTRIBUTION: &str = include_str!("../../../ATTRIBUTION.md");

/// The licence texts' file name, next to the app in every package.
pub const NOTICES_FILE: &str = "THIRD-PARTY-LICENSES.txt";

/// Most bytes read from the licence texts' file.
#[cfg(not(target_arch = "wasm32"))]
const MAX_NOTICES_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct AboutUi {
    pub open: bool,
    /// The licence texts' lines, once read (desktop), or why they could not be.
    notices: Option<Result<Vec<String>, String>>,
}

impl AboutUi {
    /// Opens the window with the licence texts shown (`--about`, for screenshots).
    pub fn open_with_notices(&mut self) {
        self.open = true;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.notices = Some(read_notices());
        }
    }
}

/// One line of `ATTRIBUTION.md` as plain text: headings without their marks, table rows
/// as cells joined by " · ", table rules and the top heading dropped. `None` drops it.
pub fn attribution_line(line: &str) -> Option<(String, bool)> {
    let t = line.trim();
    if t.starts_with("# ") {
        return None;
    }
    if let Some(h) = t.strip_prefix("## ") {
        return Some((h.to_owned(), true));
    }
    if t.starts_with('|') {
        let cells: Vec<&str> = t
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .collect();
        if cells
            .iter()
            .all(|c| c.chars().all(|ch| ch == '-' || ch == ':'))
        {
            return None;
        }
        return Some((strip_marks(&cells.join(" · ")), false));
    }
    Some((strip_marks(t), false))
}

/// Text without Markdown's code ticks and with `[text](url)` links as `text (url)`.
fn strip_marks(s: &str) -> String {
    let s = s.replace('`', "");
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(open) = rest.find('[') {
        let Some(mid) = rest[open..].find("](").map(|m| open + m) else {
            break;
        };
        let Some(close) = rest[mid..].find(')').map(|c| mid + c) else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..mid]);
        out.push_str(" (");
        out.push_str(&rest[mid + 2..close]);
        out.push(')');
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Where the packages put the licence texts, relative to the running program: next to
/// it (Windows zip and installer), in `../Resources` (the macOS app bundle) or in `..`
/// (the AppImage's `usr/`).
#[cfg(not(target_arch = "wasm32"))]
pub fn notices_candidates(exe: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Some(dir) = exe.parent() else {
        return Vec::new();
    };
    let mut out = vec![dir.join(NOTICES_FILE)];
    if let Some(up) = dir.parent() {
        out.push(up.join("Resources").join(NOTICES_FILE));
        out.push(up.join(NOTICES_FILE));
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn read_notices() -> Result<Vec<String>, String> {
    use std::io::Read;
    let exe = std::env::current_exe().map_err(|e| format!("where is the app? {e}"))?;
    for path in notices_candidates(&exe) {
        if let Ok(f) = std::fs::File::open(&path) {
            let mut text = String::new();
            f.take(MAX_NOTICES_BYTES)
                .read_to_string(&mut text)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            return Ok(text.lines().map(str::to_owned).collect());
        }
    }
    Err(format!(
        "{NOTICES_FILE} is not next to this app (a development build: \
         `cargo xtask notices` writes it to target/)."
    ))
}

impl ChamferaApp {
    /// Help > About Chamfera.
    pub(crate) fn about_window(&mut self, ctx: &egui::Context) {
        if !self.about.open {
            return;
        }
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let vr = self.viewport_rect;
        let mut close = false;
        #[cfg(not(target_arch = "wasm32"))]
        let mut load = false;
        egui::Window::new("About Chamfera")
            .id(egui::Id::unique("about window"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(egui::pos2(vr.center().x - 260.0, vr.min.y + 40.0))
            .fade_in(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(520.0);
                if widgets::dialog_title(ui, look, Icon::Body, "About Chamfera") {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!("Chamfera {}", env!("CARGO_PKG_VERSION")))
                            .size(T_TITLE)
                            .strong()
                            .color(look.c(|t| t.text)),
                    );
                    ui.label(
                        egui::RichText::new(
                            "Open-source parametric 3D CAD. MIT OR Apache-2.0, at your option.",
                        )
                        .size(T_BODY)
                        .color(look.c(|t| t.text)),
                    );
                    ui.add_space(widgets::S3);
                    egui::ScrollArea::vertical()
                        .id_salt("about scroll")
                        .max_height(300.0)
                        .show(ui, |ui| {
                            for (text, heading) in ATTRIBUTION.lines().filter_map(attribution_line)
                            {
                                if heading {
                                    ui.add_space(widgets::S3);
                                    ui.label(
                                        egui::RichText::new(text)
                                            .size(T_BODY)
                                            .strong()
                                            .color(look.c(|t| t.text)),
                                    );
                                } else if !text.is_empty() {
                                    ui.label(
                                        egui::RichText::new(text)
                                            .size(T_SMALL)
                                            .color(look.c(|t| t.text_muted)),
                                    );
                                }
                            }
                        });
                    ui.add_space(widgets::S3);
                    match &self.about.notices {
                        // Thousands of lines: only the visible rows are laid out.
                        Some(Ok(lines)) => {
                            let font = egui::FontId::monospace(T_SMALL);
                            let row = ui.fonts_mut(|f| f.row_height(&font)) + 1.0;
                            ui.spacing_mut().item_spacing.y = 1.0;
                            egui::ScrollArea::both()
                                .id_salt("about notices")
                                .max_height(220.0)
                                .auto_shrink([false, true])
                                .show_rows(ui, row, lines.len(), |ui, range| {
                                    for line in &lines[range] {
                                        // One row per line, scrolled sideways.
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(line)
                                                    .font(font.clone())
                                                    .color(look.c(|t| t.text_muted)),
                                            )
                                            .extend(),
                                        );
                                    }
                                });
                        }
                        Some(Err(e)) => {
                            ui.label(
                                egui::RichText::new(e)
                                    .size(T_SMALL)
                                    .color(look.c(|t| t.warning)),
                            );
                        }
                        None => {}
                    }
                    ui.add_space(widgets::S3);
                    #[cfg(target_arch = "wasm32")]
                    ui.hyperlink_to(
                        "Licence texts of the bundled crates",
                        format!("./{NOTICES_FILE}"),
                    );
                    #[cfg(not(target_arch = "wasm32"))]
                    if self.about.notices.is_none()
                        && widgets::secondary_button(ui, look, "Show the licence texts").clicked()
                    {
                        load = true;
                    }
                });
            });
        #[cfg(not(target_arch = "wasm32"))]
        if load {
            self.about.notices = Some(read_notices());
        }
        if close {
            self.about.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribution_markdown_reads_as_plain_lines() {
        assert_eq!(attribution_line("# Attribution"), None);
        assert_eq!(
            attribution_line("## Bundled fonts"),
            Some(("Bundled fonts".to_owned(), true))
        );
        assert_eq!(attribution_line("|---|---|"), None);
        assert_eq!(
            attribution_line("| Hack | MIT |"),
            Some(("Hack · MIT".to_owned(), false))
        );
        assert_eq!(
            attribution_line("By [resvg](https://x.org/r) and `tiny-skia`."),
            Some((
                "By resvg (https://x.org/r) and tiny-skia.".to_owned(),
                false
            ))
        );
        // An unclosed link stays as it is.
        assert_eq!(
            attribution_line("a [b](c"),
            Some(("a [b](c".to_owned(), false))
        );
    }

    #[test]
    fn the_built_in_attribution_names_the_tls_stack_and_the_fonts() {
        let text: String = ATTRIBUTION
            .lines()
            .filter_map(attribution_line)
            .map(|(t, _)| t + "\n")
            .collect();
        for name in ["rustls", "ring", "ureq", "Ubuntu", THE_NOTICES] {
            assert!(text.contains(name), "{name} missing from ATTRIBUTION.md");
        }
    }
    const THE_NOTICES: &str = NOTICES_FILE;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_licence_texts_are_looked_for_where_each_package_puts_them() {
        use std::path::Path;
        let c = notices_candidates(Path::new("/opt/app/usr/bin/chamfera"));
        assert_eq!(
            c,
            [
                Path::new("/opt/app/usr/bin/THIRD-PARTY-LICENSES.txt"),
                Path::new("/opt/app/usr/Resources/THIRD-PARTY-LICENSES.txt"),
                Path::new("/opt/app/usr/THIRD-PARTY-LICENSES.txt"),
            ]
        );
    }

    #[test]
    fn about_shows_its_window_when_open() {
        use eframe::App;
        let ctx = egui::Context::default();
        let mut app = crate::ChamferaApp::headless();
        app.about.open = true;
        for t in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 900.0),
                )),
                time: Some(f64::from(t) * 0.016),
                ..Default::default()
            };
            let mut f = eframe::Frame::_new_kittest();
            let mut out = ctx.run_ui(raw, |ui| app.ui(ui, &mut f));
            out.textures_delta.clear();
        }
        let shown = ctx.memory(|m| m.area_rect(egui::Id::unique("about window")));
        let rect = shown.expect("the About window was not drawn");
        // 520 points of content plus the body's margins on both sides.
        assert!(rect.width() >= 520.0, "{rect:?}");
    }
}
