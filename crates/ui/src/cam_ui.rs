//! The MANUFACTURE workspace: its ribbon (setup, tools, 2D and 3D operations, generate,
//! post, simulate), the tool library panel (tools with their holders, import and export),
//! the stock simulation's time slider panel,
//! the setup and operations tree in the left panel, the operation dialog (pick faces or
//! edges in the viewport, choose the tool, depths and stepover) and Post G-code in the
//! File menu. Acts only through `cam.*` commands; toolpaths are drawn by the engine as
//! viewport overlays (rapids dashed, feeds solid, theme tokens `toolpath_*`).

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use serde_json::{Value, json};

/// The Manufacture workspace's state.
#[derive(Default)]
pub struct CamUi {
    /// The MANUFACTURE tab is showing.
    pub active: bool,
    pub dialog: Option<OpDialog>,
    pub tool_dialog: Option<ToolDialog>,
    /// The Simulate panel while it is open.
    pub sim_panel: Option<SimPanel>,
    /// The Tool Library panel is open.
    pub library_open: bool,
}

/// The Simulate panel: the time slider, the resolution and the last result.
#[derive(Debug, Clone)]
pub struct SimPanel {
    /// How far through the cycle time (0 to 1).
    pub time: f64,
    pub resolution: u32,
    pub result: Option<Value>,
}

/// The operation dialog while it is open (adding, or editing `edit`).
pub struct OpDialog {
    pub kind: &'static str,
    pub edit: Option<u32>,
    pub tool: Option<u32>,
    pub face: Option<String>,
    pub edges: Vec<String>,
    /// Contour: clicks pick edges instead of a face.
    pub pick_edges: bool,
    pub stepover: String,
    pub stepdown: String,
    pub outside: bool,
    pub entry: &'static str,
    pub peck: String,
    /// Parallel: a scallop height instead of the stepover (empty for none).
    pub scallop: String,
    /// Parallel: the pass direction (degrees).
    pub angle: String,
    /// Adaptive: the engagement limit (degrees).
    pub engagement: String,
    /// 3D: material left on the part.
    pub leave: String,
    /// Parallel and adaptive: what to do where the holder would hit the part.
    pub holder_check: &'static str,
    /// Pencil: the least change of slope traced (degrees).
    pub min_angle: String,
    /// Scallop: steeper than this is a border (degrees).
    pub max_slope: String,
}

/// The Add Tool dialog, or Edit Tool for `edit`.
pub struct ToolDialog {
    pub edit: Option<u32>,
    pub kind: &'static str,
    pub name: String,
    pub diameter: String,
    pub corner: String,
    pub flute_length: String,
    pub feed: String,
    pub rpm: String,
    /// The holder: empty stickout for none.
    pub stickout: String,
    pub shank: String,
    /// Each holder segment: diameter, top diameter (empty for a cylinder), length.
    pub segments: Vec<[String; 3]>,
}

impl ToolDialog {
    fn new() -> ToolDialog {
        ToolDialog {
            edit: None,
            kind: "flat_end_mill",
            name: String::new(),
            diameter: "6 mm".into(),
            corner: String::new(),
            flute_length: String::new(),
            feed: "600".into(),
            rpm: "10000".into(),
            stickout: String::new(),
            shank: String::new(),
            segments: Vec::new(),
        }
    }

    /// The dialog on an existing tool.
    fn editing(t: &chamfera_doc::cam::Tool) -> ToolDialog {
        let mm = |v: f64| format!("{} mm", num(v));
        let h = t.holder.as_ref();
        ToolDialog {
            edit: Some(t.id),
            kind: TOOL_KINDS
                .iter()
                .find(|k| **k == t.kind.key())
                .copied()
                .unwrap_or("flat_end_mill"),
            name: t.name.clone(),
            diameter: mm(t.diameter),
            corner: if t.kind == chamfera_doc::cam::ToolKind::BullNose {
                mm(t.corner_radius)
            } else {
                String::new()
            },
            flute_length: mm(t.flute_length),
            feed: num(t.feed),
            rpm: num(t.spindle_rpm),
            stickout: h.map(|h| mm(h.stickout)).unwrap_or_default(),
            shank: h.and_then(|h| h.shank_diameter).map(mm).unwrap_or_default(),
            segments: h
                .map(|h| {
                    h.segments
                        .iter()
                        .map(|s| {
                            [
                                mm(s.diameter),
                                s.top_diameter.map(mm).unwrap_or_default(),
                                mm(s.length),
                            ]
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// The `cam.tool_add` or `cam.tool_edit` parameters.
    fn params(&self) -> Value {
        let mut p = json!({ "kind": self.kind, "diameter": self.diameter.trim() });
        if let Some(id) = self.edit {
            p["tool"] = json!(id);
        }
        if !self.name.trim().is_empty() {
            p["name"] = json!(self.name.trim());
        }
        if self.kind == "bull_nose" && !self.corner.trim().is_empty() {
            p["corner_radius"] = json!(self.corner.trim());
        }
        if !self.flute_length.trim().is_empty() {
            p["flute_length"] = json!(self.flute_length.trim());
        }
        if let Ok(f) = self.feed.trim().parse::<f64>() {
            p["feed"] = json!(f);
        }
        if let Ok(r) = self.rpm.trim().parse::<f64>() {
            p["spindle_rpm"] = json!(r);
        }
        if self.stickout.trim().is_empty() {
            if self.edit.is_some() {
                p["holder"] = Value::Null;
            }
        } else {
            let segments: Vec<Value> = self
                .segments
                .iter()
                .filter(|s| !s[0].trim().is_empty() && !s[2].trim().is_empty())
                .map(|s| {
                    let mut v = json!({ "diameter": s[0].trim(), "length": s[2].trim() });
                    if !s[1].trim().is_empty() {
                        v["top_diameter"] = json!(s[1].trim());
                    }
                    v
                })
                .collect();
            let mut h = json!({ "stickout": self.stickout.trim(), "segments": segments });
            if !self.shank.trim().is_empty() {
                h["shank_diameter"] = json!(self.shank.trim());
            }
            p["holder"] = h;
        }
        p
    }
}

/// The tool kinds the dialog offers.
const TOOL_KINDS: [&str; 5] = [
    "flat_end_mill",
    "ball_end_mill",
    "bull_nose",
    "drill",
    "chamfer_mill",
];

/// A number for a text field: at most 3 decimals, no trailing zeros.
fn num(v: f64) -> String {
    let s = format!("{:.3}", (v * 1000.0).round() / 1000.0);
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

const KINDS: [(&str, &str, Icon); 9] = [
    ("facing", "Facing", Icon::CamFacing),
    ("contour", "2D Contour", Icon::CamContour),
    ("pocket", "2D Pocket", Icon::CamPocket),
    ("drill", "Drill", Icon::CamDrill),
    ("adaptive", "Adaptive Clearing", Icon::CamAdaptive),
    ("parallel", "Parallel", Icon::CamParallel),
    ("waterline", "Waterline", Icon::CamWaterline),
    ("pencil", "Pencil", Icon::CamContour),
    ("scallop", "Scallop (experimental)", Icon::CamParallel),
];

/// The 3D operations, which have no face to pick.
fn is_3d(kind: &str) -> bool {
    matches!(
        kind,
        "adaptive" | "parallel" | "waterline" | "pencil" | "scallop"
    )
}

fn kind_icon(kind: &str) -> Icon {
    KINDS
        .iter()
        .find(|k| k.0 == kind)
        .map_or(Icon::CamGenerate, |k| k.2)
}

fn kind_title(kind: &str) -> &'static str {
    KINDS
        .iter()
        .find(|k| k.0 == kind)
        .map_or("Operation", |k| k.1)
}

impl OpDialog {
    pub fn new(kind: &'static str, tool: Option<u32>) -> OpDialog {
        OpDialog {
            kind,
            edit: None,
            tool,
            face: None,
            edges: Vec::new(),
            pick_edges: false,
            stepover: match kind {
                "facing" => "50",
                "parallel" | "scallop" => "10",
                _ => "40",
            }
            .into(),
            stepdown: match kind {
                "facing" | "waterline" => "1 mm",
                _ => "2 mm",
            }
            .into(),
            outside: true,
            entry: "helix",
            peck: String::new(),
            scallop: String::new(),
            angle: "0".into(),
            engagement: "60".into(),
            leave: match kind {
                "adaptive" => "0.3 mm",
                _ => "0 mm",
            }
            .into(),
            holder_check: "ignore",
            min_angle: "10".into(),
            max_slope: "60".into(),
        }
    }

    /// Whether viewport clicks pick something for this operation.
    fn picks(&self) -> bool {
        matches!(self.kind, "contour" | "pocket")
    }

    fn ready(&self) -> bool {
        self.tool.is_some()
            && match self.kind {
                "pocket" => self.face.is_some(),
                "contour" => self.face.is_some() || !self.edges.is_empty(),
                _ => true,
            }
    }

    /// The command parameters.
    fn params(&self) -> Value {
        let mut p = json!({});
        match self.edit {
            Some(id) => p["operation"] = json!(id),
            None => p["kind"] = json!(self.kind),
        }
        if let Some(t) = self.tool {
            p["tool"] = json!(t);
        }
        if let Ok(v) = self
            .stepover
            .trim()
            .trim_end_matches('%')
            .trim()
            .parse::<f64>()
            && matches!(self.kind, "facing" | "pocket" | "parallel" | "scallop")
            && (self.kind != "parallel" || self.scallop.trim().is_empty())
        {
            p["stepover"] = json!(v);
        }
        if !matches!(self.kind, "drill" | "parallel" | "pencil" | "scallop")
            && !self.stepdown.trim().is_empty()
        {
            p["stepdown"] = json!(self.stepdown.trim());
        }
        if matches!(self.kind, "parallel" | "adaptive") {
            p["holder_check"] = json!(self.holder_check);
        }
        let degrees = |t: &str| t.trim().trim_end_matches('°').trim().parse::<f64>().ok();
        if self.kind == "pencil"
            && let Some(a) = degrees(&self.min_angle)
        {
            p["min_angle"] = json!(a);
        }
        if self.kind == "scallop"
            && let Some(a) = degrees(&self.max_slope)
        {
            p["max_slope"] = json!(a);
        }
        if is_3d(self.kind) && !self.leave.trim().is_empty() {
            p["stock_to_leave"] = json!(self.leave.trim());
        }
        match self.kind {
            "contour" => {
                if self.pick_edges {
                    p["edges"] = json!(self.edges);
                } else if let Some(f) = &self.face {
                    p["face"] = json!(f);
                }
                p["side"] = json!(if self.outside { "outside" } else { "inside" });
            }
            "pocket" => {
                if let Some(f) = &self.face {
                    p["face"] = json!(f);
                }
                p["entry"] = json!(self.entry);
            }
            "parallel" => {
                if !self.scallop.trim().is_empty() {
                    p["scallop"] = json!(self.scallop.trim());
                }
                if let Ok(a) = self
                    .angle
                    .trim()
                    .trim_end_matches('°')
                    .trim()
                    .parse::<f64>()
                {
                    p["angle"] = json!(a);
                }
            }
            "adaptive" => {
                if let Ok(a) = self
                    .engagement
                    .trim()
                    .trim_end_matches('°')
                    .trim()
                    .parse::<f64>()
                {
                    p["max_engagement"] = json!(a);
                }
            }
            "drill" => {
                if !self.peck.trim().is_empty() {
                    p["peck"] = json!(self.peck.trim());
                } else if self.edit.is_some() {
                    p["peck"] = Value::Null;
                }
            }
            _ => {}
        }
        p
    }
}

impl ChamferaApp {
    /// Opens or closes the Manufacture workspace (toolpaths show only there).
    pub(crate) fn set_manufacture(&mut self, on: bool) {
        self.cam.active = on;
        if !on {
            self.cam.dialog = None;
            self.cam.tool_dialog = None;
            self.cam.library_open = false;
            if self.cam.sim_panel.take().is_some() {
                self.run("cam.simulate", json!({ "clear": true }));
            }
        }
        self.run("cam.show", json!({ "visible": on }));
    }

    /// The last tool of the library, for a new operation.
    fn last_tool(&self) -> Option<u32> {
        self.session.doc.cam.tools.last().map(|t| t.id)
    }

    /// Setup: stock from the model's box, part zero at the stock's top corner.
    pub(crate) fn cam_setup(&mut self) {
        if let Some(r) = self.run("cam.setup", json!({})) {
            self.message = Some((
                format!("Added {}", r["name"].as_str().unwrap_or("a setup")),
                false,
            ));
        }
    }

    /// Add Tool: opens its dialog with a 6 mm flat end mill filled in.
    pub(crate) fn open_cam_tool_dialog(&mut self) {
        self.cam.tool_dialog = Some(ToolDialog::new());
    }

    /// Opens the dialog for a new operation of this kind (`facing`, `contour`, `pocket`,
    /// `drill`), with the last tool of the library.
    pub(crate) fn open_cam_op(&mut self, kind: &'static str) {
        self.cam.dialog = Some(OpDialog::new(kind, self.last_tool()));
    }

    pub(crate) fn generate_toolpaths(&mut self) {
        if let Some(r) = self.run("cam.generate", json!({})) {
            let errors = r["errors"].as_u64().unwrap_or(0);
            let minutes = r["minutes"].as_f64().unwrap_or(0.0);
            self.message = Some(if errors == 0 {
                (
                    format!("Toolpaths generated · cycle time {minutes:.1} min"),
                    false,
                )
            } else {
                (
                    format!("{errors} operation(s) could not be generated: see the tree"),
                    true,
                )
            });
        }
    }

    /// File > Post G-code: asks where, then runs `cam.post`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn post_dialog(&mut self, dialect: &str) {
        let stem = self
            .session
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or_else(|| "design".to_owned(), |s| s.to_string_lossy().into_owned());
        let Some(mut path) = rfd::FileDialog::new()
            .set_title("Post G-code")
            .add_filter("G-code", &["nc", "ngc", "gcode"])
            .set_file_name(format!("{stem}.nc"))
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("nc");
        }
        if let Some(r) = self.run(
            "cam.post",
            json!({ "path": path.to_string_lossy(), "dialect": dialect }),
        ) {
            self.message = Some((
                format!("Posted {} lines to {}", r["lines"], path.display()),
                false,
            ));
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn post_dialog(&mut self, _dialect: &str) {
        self.message = Some(("Posting G-code is in the desktop app for now".into(), true));
    }

    /// The left panel in Manufacture: setups with their operations, then the tools.
    pub(crate) fn cam_tree(&mut self, ui: &mut egui::Ui) {
        let look = self.look();
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            widgets::caption(ui, look, "MANUFACTURE");
        });
        ui.add_space(widgets::S2);
        let cam = self.session.doc.cam.clone();
        let mut open: Option<u32> = None;
        let mut toggle: Option<(u32, bool)> = None;
        let mut delete: Option<u32> = None;
        let mut edit_tool: Option<u32> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if cam.setups.is_empty() {
                    ui.label(
                        egui::RichText::new("No setup yet: click Setup in the ribbon")
                            .size(widgets::T_SMALL)
                            .color(look.c(|t| t.text_muted)),
                    );
                }
                for st in &cam.setups {
                    widgets::menu_item(ui, look, Icon::CamSetup, &st.name, true);
                    for o in cam.operations.iter().filter(|o| o.setup == st.id) {
                        let stale = self.session.toolpath_stale(o.id);
                        let status = match self.session.toolpath(o.id) {
                            _ if stale => "regenerating…".to_owned(),
                            Some(Ok(tp)) => {
                                format!("{:.1} min", tp.stats(st.rapid_rate).minutes)
                            }
                            Some(Err(e)) if o.suppressed => {
                                let _ = e;
                                "off".to_owned()
                            }
                            Some(Err(_)) => "error".to_owned(),
                            None => String::new(),
                        };
                        let tip = match self.session.toolpath(o.id) {
                            Some(Err(e)) => e.clone(),
                            _ => format!("{} · {status}", kind_title(o.op.key())),
                        };
                        let colour = if status == "error" {
                            look.c(|t| t.error)
                        } else {
                            look.c(|t| t.text_muted)
                        };
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if widgets::icon_button(
                                        ui,
                                        look,
                                        Icon::Delete,
                                        22.0,
                                        14.0,
                                        true,
                                        false,
                                    )
                                    .on_hover_text(format!(
                                        "{}\nRemove it from the setup",
                                        label("cam.op_delete")
                                    ))
                                    .clicked()
                                    {
                                        delete = Some(o.id);
                                    }
                                    let eye = if o.suppressed {
                                        Icon::EyeOff
                                    } else {
                                        Icon::Eye
                                    };
                                    if widgets::icon_button(ui, look, eye, 22.0, 14.0, true, false)
                                        .on_hover_text(if o.suppressed {
                                            "Restore\nGenerate and post this operation again"
                                        } else {
                                            "Suppress\nSkip this operation when generating and posting"
                                        })
                                        .clicked()
                                    {
                                        toggle = Some((o.id, !o.suppressed));
                                    }
                                    ui.label(
                                        egui::RichText::new(&status)
                                            .size(widgets::T_SMALL)
                                            .color(colour),
                                    );
                                    ui.with_layout(
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            let r = widgets::menu_item(
                                                ui,
                                                look,
                                                kind_icon(o.op.key()),
                                                &o.name,
                                                !o.suppressed,
                                            );
                                            if r.on_hover_text(tip).clicked() {
                                                open = Some(o.id);
                                            }
                                        },
                                    );
                                },
                            );
                        });
                    }
                }
                ui.add_space(widgets::S3);
                widgets::caption(ui, look, "TOOLS");
                for t in &cam.tools {
                    let r = widgets::menu_item(
                        ui,
                        look,
                        Icon::CamTool,
                        &format!("T{} {}", t.number, t.name),
                        true,
                    );
                    if r.on_hover_text("Edit Tool\nIts size, feeds and holder").clicked() {
                        edit_tool = Some(t.id);
                    }
                }
            });
        if let Some(id) = open {
            self.edit_operation(id);
        }
        if let Some((id, s)) = toggle {
            self.run("cam.op_edit", json!({ "operation": id, "suppressed": s }));
        }
        if let Some(id) = delete {
            self.run("cam.op_delete", json!({ "operation": id }));
        }
        if let Some(t) = edit_tool.and_then(|id| self.session.doc.cam.tool(id)) {
            self.cam.tool_dialog = Some(ToolDialog::editing(t));
        }
    }

    /// Opens the dialog on an existing operation.
    fn edit_operation(&mut self, id: u32) {
        use chamfera_doc::cam::{Entry, OpKind, Side};
        let Some(o) = self.session.doc.cam.operation(id).cloned() else {
            return;
        };
        let kind = KINDS
            .iter()
            .find(|k| k.0 == o.op.key())
            .map_or("facing", |k| k.0);
        let mut d = OpDialog::new(kind, Some(o.tool));
        d.edit = Some(id);
        let pct = |v: f64| num(v * 100.0);
        let mm = |v: f64| format!("{} mm", num(v));
        match o.op {
            OpKind::Facing { stepover, stepdown } => {
                d.stepover = pct(stepover);
                d.stepdown = mm(stepdown);
            }
            OpKind::Contour {
                face,
                edges,
                side,
                stepdown,
                ..
            } => {
                d.pick_edges = face.is_none();
                d.face = face;
                d.edges = edges;
                d.outside = side == Side::Right;
                d.stepdown = mm(stepdown);
            }
            OpKind::Pocket {
                face,
                stepover,
                stepdown,
                entry,
                ..
            } => {
                d.face = Some(face);
                d.stepover = pct(stepover);
                d.stepdown = mm(stepdown);
                d.entry = match entry {
                    Entry::Helix => "helix",
                    Entry::Ramp => "ramp",
                    Entry::Plunge => "plunge",
                };
            }
            OpKind::Drill { peck, .. } => {
                d.peck = peck.map(mm).unwrap_or_default();
            }
            OpKind::Parallel {
                stepover,
                scallop,
                angle,
                stock_to_leave,
                holder_check,
                ..
            } => {
                d.stepover = pct(stepover);
                d.scallop = scallop.map(mm).unwrap_or_default();
                d.angle = num(angle.to_degrees());
                d.leave = mm(stock_to_leave);
                d.holder_check = holder_check.key();
            }
            OpKind::Pencil {
                stock_to_leave,
                min_angle,
                ..
            } => {
                d.leave = mm(stock_to_leave);
                d.min_angle = num(min_angle.to_degrees());
            }
            OpKind::Scallop {
                stepover,
                stock_to_leave,
                max_slope,
                ..
            } => {
                d.stepover = pct(stepover);
                d.leave = mm(stock_to_leave);
                d.max_slope = num(max_slope.to_degrees());
            }
            OpKind::Waterline {
                stepdown,
                stock_to_leave,
                ..
            } => {
                d.stepdown = mm(stepdown);
                d.leave = mm(stock_to_leave);
            }
            OpKind::Adaptive {
                stepdown,
                max_engagement,
                stock_to_leave,
                holder_check,
            } => {
                d.stepdown = mm(stepdown);
                d.engagement = num(max_engagement.to_degrees());
                d.leave = mm(stock_to_leave);
                d.holder_check = holder_check.key();
            }
        }
        self.cam.dialog = Some(d);
    }

    /// True when a viewport click goes to the operation dialog.
    pub(crate) fn cam_picking(&self) -> bool {
        self.cam.active && self.cam.dialog.as_ref().is_some_and(OpDialog::picks)
    }

    /// A viewport click while the dialog picks: a face (its name), or for a contour in
    /// edge mode an edge (click again to unpick).
    pub(crate) fn cam_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let edge_mode = self.cam.dialog.as_ref().is_some_and(|d| d.pick_edges);
        if edge_mode {
            let Some((_, _, name)) = self.session.edge_at(x, y, w, h) else {
                return;
            };
            if let Some(d) = self.cam.dialog.as_mut() {
                if let Some(i) = d.edges.iter().position(|e| *e == name) {
                    d.edges.remove(i);
                } else {
                    d.edges.push(name);
                }
            }
            return;
        }
        let Some((feature, role, _)) = self.session.face_at(x, y, w, h) else {
            self.message = Some(("Click a flat face of the model".into(), true));
            return;
        };
        let name = chamfera_engine::solids::face_key(feature, &role);
        if let Some(d) = self.cam.dialog.as_mut() {
            d.face = Some(name);
        }
    }

    /// The operation dialog and the Add Tool dialog.
    pub(crate) fn cam_dialogs(&mut self, ctx: &egui::Context) {
        if !self.cam.active {
            return;
        }
        self.library_window(ctx);
        self.tool_window(ctx);
        self.sim_window(ctx);
        const WIDTH: f32 = 320.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let tools: Vec<(u32, String)> = self
            .session
            .doc
            .cam
            .tools
            .iter()
            .map(|t| (t.id, format!("T{} {}", t.number, t.name)))
            .collect();
        let Some(d) = self.cam.dialog.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let title = kind_title(d.kind);
        let ready = d.ready();
        egui::Window::new(title)
            .id(egui::Id::unique("cam operation dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(WIDTH)
            .min_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, kind_icon(d.kind), title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("cam op grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            widgets::field_label(ui, look, "Tool");
                            let current = tools
                                .iter()
                                .find(|t| Some(t.0) == d.tool)
                                .map_or("Pick a tool", |t| t.1.as_str());
                            egui::ComboBox::from_id_salt("cam tool")
                                .selected_text(current)
                                .width(widgets::FIELD_WIDTH)
                                .show_ui(ui, |ui| {
                                    for (id, text) in &tools {
                                        ui.selectable_value(&mut d.tool, Some(*id), text);
                                    }
                                });
                            ui.end_row();
                            if d.kind == "contour" {
                                widgets::field_label(ui, look, "Follow");
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut d.pick_edges, false, "Face");
                                    ui.radio_value(&mut d.pick_edges, true, "Edges");
                                });
                                ui.end_row();
                            }
                            if d.picks() {
                                let edges = d.kind == "contour" && d.pick_edges;
                                widgets::field_label(
                                    ui,
                                    look,
                                    if edges { "Edges" } else { "Face" },
                                );
                                let text = if edges {
                                    match d.edges.len() {
                                        0 => "Click edges".to_owned(),
                                        n => format!("{n} selected"),
                                    }
                                } else {
                                    d.face.clone().unwrap_or_else(|| {
                                        if d.kind == "pocket" {
                                            "Click the pocket floor".into()
                                        } else {
                                            "Click a flat face".into()
                                        }
                                    })
                                };
                                ui.label(text);
                                ui.end_row();
                            }
                            if d.kind == "contour" {
                                widgets::field_label(ui, look, "Side");
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut d.outside, true, "Outside");
                                    ui.radio_value(&mut d.outside, false, "Inside");
                                });
                                ui.end_row();
                            }
                            if matches!(d.kind, "facing" | "pocket" | "parallel" | "scallop") {
                                widgets::field_label(
                                    ui,
                                    look,
                                    if d.kind == "scallop" {
                                        "Stepover % (along)"
                                    } else {
                                        "Stepover %"
                                    },
                                );
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut d.stepover),
                                );
                                ui.end_row();
                            }
                            if d.kind == "parallel" {
                                widgets::field_label(ui, look, "Scallop");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut d.scallop)
                                        .hint_text("ball mill: cusp height"),
                                );
                                ui.end_row();
                                widgets::field_label(ui, look, "Angle °");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut d.angle),
                                );
                                ui.end_row();
                            }
                            if d.kind == "adaptive" {
                                widgets::field_label(ui, look, "Engagement °");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut d.engagement),
                                );
                                ui.end_row();
                            }
                            if d.kind == "pencil" {
                                widgets::field_label(ui, look, "Corner angle °");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut d.min_angle),
                                );
                                ui.end_row();
                            }
                            if d.kind == "scallop" {
                                widgets::field_label(ui, look, "Steepest °");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut d.max_slope),
                                );
                                ui.end_row();
                            }
                            if matches!(d.kind, "parallel" | "adaptive") {
                                widgets::field_label(ui, look, "Holder");
                                egui::ComboBox::from_id_salt("cam holder check")
                                    .selected_text(d.holder_check)
                                    .width(widgets::FIELD_WIDTH)
                                    .show_ui(ui, |ui| {
                                        for k in ["ignore", "lift", "refuse"] {
                                            ui.selectable_value(&mut d.holder_check, k, k);
                                        }
                                    });
                                ui.end_row();
                            }
                            if is_3d(d.kind) {
                                widgets::field_label(ui, look, "Stock to leave");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut d.leave),
                                );
                                ui.end_row();
                            }
                            if !matches!(d.kind, "drill" | "parallel" | "pencil" | "scallop") {
                                widgets::field_label(ui, look, "Stepdown");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut d.stepdown),
                                );
                                ui.end_row();
                            }
                            if d.kind == "pocket" {
                                widgets::field_label(ui, look, "Entry");
                                ui.horizontal(|ui| {
                                    for e in ["helix", "ramp", "plunge"] {
                                        ui.radio_value(&mut d.entry, e, e);
                                    }
                                });
                                ui.end_row();
                            }
                            if d.kind == "drill" {
                                widgets::field_label(ui, look, "Peck");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut d.peck).hint_text("none (G81)"),
                                );
                                ui.end_row();
                            }
                        });
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, "OK", ready).clicked() {
                        ok = true;
                    }
                    if widgets::secondary_button(ui, look, "Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.egui_wants_keyboard_input() {
            cancel = true;
        }
        if ok && ready {
            let (id, params) = match d.edit {
                Some(_) => ("cam.op_edit", d.params()),
                None => ("cam.op_add", d.params()),
            };
            if let Some(r) = self.run(id, params) {
                self.message = Some(match r.get("error").and_then(Value::as_str) {
                    Some(e) => (
                        format!("{}: {e}", r["name"].as_str().unwrap_or(title)),
                        true,
                    ),
                    None => (
                        format!(
                            "{} · {:.1} min",
                            r["name"].as_str().unwrap_or(title),
                            r["minutes"].as_f64().unwrap_or(0.0)
                        ),
                        false,
                    ),
                });
                self.cam.dialog = None;
            }
        }
        if cancel {
            self.cam.dialog = None;
        }
    }

    /// Simulate: opens the panel and simulates the whole cycle, or closes it and takes
    /// the stock away.
    pub(crate) fn toggle_simulation(&mut self) {
        if self.cam.sim_panel.take().is_some() {
            self.run("cam.simulate", json!({ "clear": true }));
            return;
        }
        let mut panel = SimPanel {
            time: 1.0,
            resolution: 256,
            result: None,
        };
        panel.result = self.run(
            "cam.simulate",
            json!({ "time": 1.0, "resolution": panel.resolution }),
        );
        if panel.result.is_some() {
            self.cam.sim_panel = Some(panel);
        }
    }

    /// Runs the simulation again at the panel's time and resolution.
    pub(crate) fn resimulate(&mut self) {
        let Some(p) = self.cam.sim_panel.clone() else {
            return;
        };
        let r = self.run(
            "cam.simulate",
            json!({ "time": p.time, "resolution": p.resolution }),
        );
        if let Some(panel) = self.cam.sim_panel.as_mut() {
            panel.result = r;
        }
    }

    /// The Simulate panel: a time slider through the cycle, the resolution, and what the
    /// stock shows (removed and rest volume, gouges).
    fn sim_window(&mut self, ctx: &egui::Context) {
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let units = self.units();
        let pos = self.tool_dialog_pos(300.0);
        let Some(p) = self.cam.sim_panel.as_mut() else {
            return;
        };
        let (mut changed, mut close) = (false, false);
        egui::Window::new("Simulate")
            .id(egui::Id::unique("cam simulate panel"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(300.0)
            .resizable(false)
            .show(ctx, |ui| {
                if widgets::dialog_title(ui, look, Icon::CamSimulate, label("cam.simulate")) {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("cam sim grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            widgets::field_label(ui, look, "Time");
                            let mut pct = p.time * 100.0;
                            let r = ui.add(
                                egui::Slider::new(&mut pct, 0.0..=100.0)
                                    .suffix(" %")
                                    .fixed_decimals(0),
                            );
                            if r.changed() {
                                p.time = (pct / 100.0).clamp(0.0, 1.0);
                                changed = true;
                            }
                            ui.end_row();
                            widgets::field_label(ui, look, "Resolution");
                            egui::ComboBox::from_id_salt("cam sim resolution")
                                .selected_text(format!("{} columns", p.resolution))
                                .width(widgets::FIELD_WIDTH)
                                .show_ui(ui, |ui| {
                                    for n in [128u32, 256, 512, 1024] {
                                        if ui
                                            .selectable_value(
                                                &mut p.resolution,
                                                n,
                                                format!("{n} columns"),
                                            )
                                            .changed()
                                        {
                                            changed = true;
                                        }
                                    }
                                });
                            ui.end_row();
                            if let Some(r) = &p.result {
                                let v = |k: &str| r[k].as_f64().unwrap_or(0.0);
                                widgets::field_label(ui, look, "Removed");
                                ui.label(units.volume(v("removed_volume")));
                                ui.end_row();
                                widgets::field_label(ui, look, "Rest material");
                                ui.label(units.volume(v("rest_volume")));
                                ui.end_row();
                                widgets::field_label(ui, look, "Gouges");
                                let n = r["gouges"]["cells"].as_u64().unwrap_or(0);
                                if n == 0 {
                                    ui.label("none");
                                } else {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{n} columns, deepest {}",
                                            units.length(
                                                r["gouges"]["max_depth"].as_f64().unwrap_or(0.0)
                                            )
                                        ))
                                        .color(look.c(|t| t.gouge)),
                                    );
                                }
                                ui.end_row();
                                if r["holder"]["checked"].as_bool() == Some(true) {
                                    widgets::field_label(ui, look, "Holder");
                                    let n = r["holder"]["moves"].as_u64().unwrap_or(0);
                                    if n == 0 {
                                        ui.label("clear");
                                    } else {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{n} moves collide, deepest {}",
                                                units.length(
                                                    r["holder"]["max_depth"]
                                                        .as_f64()
                                                        .unwrap_or(0.0)
                                                )
                                            ))
                                            .color(look.c(|t| t.gouge)),
                                        );
                                    }
                                    ui.end_row();
                                }
                            }
                        });
                });
                widgets::dialog_footer(ui, look, |ui| {
                    close |= widgets::secondary_button(ui, look, "Close").clicked();
                });
            });
        if changed {
            self.resimulate();
        }
        if close {
            self.cam.sim_panel = None;
            self.run("cam.simulate", json!({ "clear": true }));
        }
    }

    fn tool_window(&mut self, ctx: &egui::Context) {
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let pos = self.tool_dialog_pos(320.0);
        let Some(t) = self.cam.tool_dialog.as_mut() else {
            return;
        };
        let (mut ok, mut cancel) = (false, false);
        let title = label(if t.edit.is_some() {
            "cam.tool_edit"
        } else {
            "cam.tool_add"
        });
        egui::Window::new(title)
            .id(egui::Id::unique("cam tool dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(320.0)
            .resizable(false)
            .show(ctx, |ui| {
                if widgets::dialog_title(ui, look, Icon::CamTool, title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("cam tool grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            widgets::field_label(ui, look, "Kind");
                            egui::ComboBox::from_id_salt("cam tool kind")
                                .selected_text(t.kind.replace('_', " "))
                                .width(widgets::FIELD_WIDTH)
                                .show_ui(ui, |ui| {
                                    for k in TOOL_KINDS {
                                        ui.selectable_value(&mut t.kind, k, k.replace('_', " "));
                                    }
                                });
                            ui.end_row();
                            let mut rows: Vec<(&str, &mut String)> =
                                vec![("Name", &mut t.name), ("Diameter", &mut t.diameter)];
                            if t.kind == "bull_nose" {
                                rows.push(("Corner radius", &mut t.corner));
                            }
                            rows.push(("Flute length", &mut t.flute_length));
                            rows.push(("Feed mm/min", &mut t.feed));
                            rows.push(("Spindle rpm", &mut t.rpm));
                            rows.push(("Stickout", &mut t.stickout));
                            rows.push(("Shank Ø", &mut t.shank));
                            for (text, field) in rows {
                                widgets::field_label(ui, look, text);
                                ui.add_sized(widgets::FIELD, egui::TextEdit::singleline(field));
                                ui.end_row();
                            }
                        });
                    ui.add_space(widgets::S2);
                    widgets::caption(ui, look, "HOLDER (BOTTOM FIRST): Ø, TOP Ø, LENGTH");
                    let mut remove = None;
                    for (i, seg) in t.segments.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            for (k, hint) in ["Ø", "top Ø", "length"].iter().enumerate() {
                                ui.add_sized(
                                    [76.0, widgets::ROW_HEIGHT],
                                    egui::TextEdit::singleline(&mut seg[k]).hint_text(*hint),
                                );
                            }
                            if widgets::icon_button(ui, look, Icon::Delete, 22.0, 14.0, true, false)
                                .on_hover_text("Remove this segment")
                                .clicked()
                            {
                                remove = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        t.segments.remove(i);
                    }
                    if widgets::secondary_button(ui, look, "Add Segment").clicked() {
                        t.segments
                            .push([String::new(), String::new(), String::new()]);
                    }
                });
                widgets::dialog_footer(ui, look, |ui| {
                    ok = widgets::primary_button(ui, look, "OK", true).clicked();
                    cancel |= widgets::secondary_button(ui, look, "Cancel").clicked();
                });
            });
        if ok {
            let (id, p) = match t.edit {
                Some(_) => ("cam.tool_edit", t.params()),
                None => ("cam.tool_add", t.params()),
            };
            if let Some(r) = self.run(id, p) {
                self.message = Some((
                    format!(
                        "{} T{} {}",
                        if id == "cam.tool_add" {
                            "Added"
                        } else {
                            "Changed"
                        },
                        r["number"],
                        r["name"].as_str().unwrap_or("")
                    ),
                    false,
                ));
                self.cam.tool_dialog = None;
            }
        }
        if cancel {
            self.cam.tool_dialog = None;
        }
    }

    /// The Tool Library panel: every tool with its size and holder, to edit or delete,
    /// and libraries to import or export.
    fn library_window(&mut self, ctx: &egui::Context) {
        if !self.cam.library_open {
            return;
        }
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let units = self.units();
        let pos = self.tool_dialog_pos(380.0);
        let tools = self.session.doc.cam.tools.clone();
        let (mut close, mut edit, mut delete, mut add) = (false, None, None, false);
        let (mut import, mut export) = (false, false);
        egui::Window::new("Tool Library")
            .id(egui::Id::unique("cam tool library"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(380.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(380.0);
                if widgets::dialog_title(ui, look, Icon::CamTool, "Tool Library") {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    if tools.is_empty() {
                        ui.label(
                            egui::RichText::new("No tools yet: add one, or import a library")
                                .size(widgets::T_SMALL)
                                .color(look.c(|t| t.text_muted)),
                        );
                    }
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for t in &tools {
                                ui.horizontal(|ui| {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if widgets::icon_button(
                                                ui,
                                                look,
                                                Icon::Delete,
                                                22.0,
                                                14.0,
                                                true,
                                                false,
                                            )
                                            .on_hover_text(format!(
                                                "{}\nRemove it from the library",
                                                label("cam.tool_delete")
                                            ))
                                            .clicked()
                                            {
                                                delete = Some(t.id);
                                            }
                                            let holder = match &t.holder {
                                                Some(h) => format!(
                                                    "out {}, {} piece holder",
                                                    units.length(h.stickout),
                                                    h.segments.len()
                                                ),
                                                None => "no holder".to_owned(),
                                            };
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "Ø{} · {holder}",
                                                    units.length(t.diameter)
                                                ))
                                                .size(widgets::T_SMALL)
                                                .color(look.c(|t| t.text_muted)),
                                            );
                                            ui.with_layout(
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    if widgets::menu_item(
                                                        ui,
                                                        look,
                                                        Icon::CamTool,
                                                        &format!("T{} {}", t.number, t.name),
                                                        true,
                                                    )
                                                    .on_hover_text(format!(
                                                        "{}\nIts size, feeds and holder",
                                                        label("cam.tool_edit")
                                                    ))
                                                    .clicked()
                                                    {
                                                        edit = Some(t.id);
                                                    }
                                                },
                                            );
                                        },
                                    );
                                });
                            }
                        });
                });
                widgets::dialog_footer(ui, look, |ui| {
                    add = widgets::primary_button(ui, look, "Add Tool", true).clicked();
                    close |= widgets::secondary_button(ui, look, "Close").clicked();
                    export = widgets::secondary_button(ui, look, "Export…").clicked();
                    import = widgets::secondary_button(ui, look, "Import…").clicked();
                });
            });
        if let Some(t) = edit.and_then(|id| self.session.doc.cam.tool(id)) {
            self.cam.tool_dialog = Some(ToolDialog::editing(t));
        }
        if let Some(id) = delete {
            self.run("cam.tool_delete", json!({ "tool": id }));
        }
        if add {
            self.open_cam_tool_dialog();
        }
        if import {
            self.tool_library_file(false);
        }
        if export {
            self.tool_library_file(true);
        }
        if close {
            self.cam.library_open = false;
        }
    }

    /// Asks for a tool library file and imports or exports it.
    #[cfg(not(target_arch = "wasm32"))]
    fn tool_library_file(&mut self, export: bool) {
        let d = rfd::FileDialog::new().add_filter("Tool library", &["json"]);
        let path = if export {
            d.set_title(label("cam.tool_library_export"))
                .set_file_name("tools.json")
                .save_file()
        } else {
            d.set_title(label("cam.tool_library_import")).pick_file()
        };
        let Some(path) = path else {
            return;
        };
        let id = if export {
            "cam.tool_library_export"
        } else {
            "cam.tool_library_import"
        };
        if let Some(r) = self.run(id, json!({ "path": path.to_string_lossy() })) {
            let n = r["tools"]
                .as_u64()
                .or_else(|| r["tools"].as_array().map(|a| a.len() as u64))
                .unwrap_or(0);
            self.message = Some((
                format!(
                    "{} {n} tools {} {}",
                    if export { "Exported" } else { "Imported" },
                    if export { "to" } else { "from" },
                    path.display()
                ),
                false,
            ));
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn tool_library_file(&mut self, _export: bool) {
        self.message = Some((
            "Tool library files are in the desktop app for now".into(),
            true,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pocket_dialog_adds_a_pocket_on_the_picked_floor() {
        let mut app = ChamferaApp::headless();
        app.run("sketch.create", json!({ "plane": "xy" }));
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 60, "y2": 40 }),
        );
        app.run("sketch.finish", json!({}));
        app.run(
            "solid.extrude",
            json!({ "sketch": 1, "profiles": [{ "x": 5, "y": 5 }], "distance": 10 }),
        );
        app.run(
            "sketch.create_on_face",
            json!({ "feature": 2, "face": "top" }),
        );
        app.run(
            "sketch.rectangle",
            json!({ "x1": 10, "y1": 10, "x2": 50, "y2": 30 }),
        );
        app.run("sketch.finish", json!({}));
        app.run(
            "solid.extrude",
            json!({ "sketch": 3, "profiles": [{ "x": 30, "y": 20 }], "distance": 4,
                    "direction": "reversed", "operation": "cut" }),
        );
        app.set_manufacture(true);
        app.run(
            "cam.tool_add",
            json!({ "kind": "flat_end_mill", "diameter": 6 }),
        );
        app.cam.dialog = Some(OpDialog::new("pocket", app.last_tool()));
        // Look straight down: the pocket floor (z = 6) fills the middle of the view.
        let (w, h) = (800.0, 600.0);
        app.run("view.standard", json!({ "view": "top" }));
        app.run("view.fit", json!({ "width": w, "height": h }));
        assert!(app.cam_picking());
        app.cam_click(w / 2.0, h / 2.0, w, h);
        let d = app.cam.dialog.as_ref().unwrap();
        assert_eq!(d.face.as_deref(), Some("4:top"));
        let r = app.run("cam.op_add", d.params()).unwrap();
        assert!(r.get("error").is_none(), "{r}");
        // The toolpath shows as overlays in both styles.
        let styles: Vec<_> = app.session.scene.overlays.iter().map(|o| o.style).collect();
        assert!(styles.contains(&chamfera_render::OverlayStyle::ToolpathFeed));
        assert!(styles.contains(&chamfera_render::OverlayStyle::ToolpathRapid));
        // Leaving Manufacture hides them.
        app.set_manufacture(false);
        assert!(
            !app.session
                .scene
                .overlays
                .iter()
                .any(|o| o.style == chamfera_render::OverlayStyle::ToolpathFeed)
        );
    }
}
