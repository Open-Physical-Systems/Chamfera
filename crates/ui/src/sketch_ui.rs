//! Sketch mode: the sketch toolbar, drawing tools with previews, selection, constraint and
//! dimension buttons, dimension labels with inline editing, and point dragging.
//!
//! Like the rest of the UI it acts only through commands (`sketch.*`).

use crate::icons::Icon;
use crate::widgets::{self, menu_item};
use crate::{ChamferaApp, color, label};
use chamfera_engine::sketching::{SketchItem, curve_polyline};
use chamfera_geom::{Point2, Point3};
use chamfera_render::{Overlay, OverlayStyle, mat_mul, transform_point};
use chamfera_sketch::{CurveId, OffsetCorners, PointId, ToolOp};
use egui::{Pos2, Rect, Vec2};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Line,
    Rectangle,
    CentreRectangle,
    Circle,
    Arc,
    ThreePointArc,
    TangentArc,
    Offset,
    Mirror,
    RectangularPattern,
    CircularPattern,
    Fillet,
    Chamfer,
    Trim,
    Extend,
    Split,
}

/// The CREATE tools shown in the ribbon; the group's list has all of `CREATE_ALL`.
const CREATE_SHOWN: [Tool; 6] = [
    Tool::Line,
    Tool::Rectangle,
    Tool::Circle,
    Tool::Arc,
    Tool::Offset,
    Tool::Mirror,
];
pub(crate) const CREATE_ALL: [Tool; 11] = [
    Tool::Line,
    Tool::Rectangle,
    Tool::CentreRectangle,
    Tool::Circle,
    Tool::Arc,
    Tool::ThreePointArc,
    Tool::TangentArc,
    Tool::Offset,
    Tool::Mirror,
    Tool::RectangularPattern,
    Tool::CircularPattern,
];
/// The MODIFY tools shown in the ribbon (Construction and Delete follow them).
pub(crate) const MODIFY_TOOLS: [Tool; 5] = [
    Tool::Fillet,
    Tool::Chamfer,
    Tool::Trim,
    Tool::Extend,
    Tool::Split,
];

impl Tool {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Line => label("sketch.line"),
            Tool::Rectangle => label("sketch.rectangle"),
            Tool::CentreRectangle => label("sketch.rectangle.centre"),
            Tool::Circle => label("sketch.circle"),
            Tool::Arc => label("sketch.arc"),
            Tool::ThreePointArc => label("sketch.arc.three_point"),
            Tool::TangentArc => label("sketch.arc.tangent"),
            Tool::Offset => label("sketch.offset"),
            Tool::Mirror => label("sketch.mirror"),
            Tool::RectangularPattern => label("sketch.pattern.rectangular"),
            Tool::CircularPattern => label("sketch.pattern.circular"),
            // Short ribbon labels; the command labels say "Sketch Fillet" to tell them
            // from solid fillets.
            Tool::Fillet => "Fillet",
            Tool::Chamfer => "Chamfer",
            Tool::Trim => label("sketch.trim"),
            Tool::Extend => label("sketch.extend"),
            Tool::Split => label("sketch.split"),
        }
    }

    pub(crate) fn icon(self) -> Icon {
        match self {
            Tool::Select => Icon::Select,
            Tool::Line => Icon::Line,
            Tool::Rectangle => Icon::Rectangle,
            Tool::CentreRectangle => Icon::RectangleCentre,
            Tool::Circle => Icon::Circle,
            Tool::Arc => Icon::Arc,
            Tool::ThreePointArc => Icon::ArcThreePoint,
            Tool::TangentArc => Icon::ArcTangent,
            Tool::Offset => Icon::Offset,
            Tool::Mirror => Icon::Mirror,
            Tool::RectangularPattern => Icon::PatternRectangular,
            Tool::CircularPattern => Icon::PatternCircular,
            Tool::Fillet => Icon::Fillet,
            Tool::Chamfer => Icon::Chamfer,
            Tool::Trim => Icon::Trim,
            Tool::Extend => Icon::Extend,
            Tool::Split => Icon::Split,
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Tool::Select => {
                "Select
Pick points and curves to constrain or dimension (Shift adds); drag points to move them"
            }
            Tool::Line => {
                "Line
Click point after point; click the first point again to close the shape"
            }
            Tool::Rectangle => {
                "Rectangle
Two clicks: opposite corners"
            }
            Tool::CentreRectangle => {
                "Centre Rectangle
Two clicks: the centre, then a corner"
            }
            Tool::Circle => {
                "Circle
Two clicks: the centre, then a point on the circle"
            }
            Tool::Arc => {
                "Arc
Three clicks: the centre, the start, then the end (counter-clockwise)"
            }
            Tool::ThreePointArc => {
                "3-Point Arc
Three clicks: the start, the end, then a point the arc passes through"
            }
            Tool::TangentArc => {
                "Tangent Arc
Click the free end of a line or arc, then where the arc ends; it leaves smoothly"
            }
            Tool::Offset => {
                "Offset
Click a curve to offset its whole chain, then click the side; the distance is in the tool box"
            }
            Tool::Mirror => {
                "Mirror
Select the geometry first, then click the line to mirror it in; the copy stays symmetric"
            }
            Tool::RectangularPattern => {
                "Rectangular Pattern
Select the geometry, set columns, rows and spacing in the tool box, then Apply"
            }
            Tool::CircularPattern => {
                "Circular Pattern
Select the geometry, set the count in the tool box, then click the centre"
            }
            Tool::Fillet => {
                "Fillet
Click a corner where two lines meet to round it; the radius is in the tool box"
            }
            Tool::Chamfer => {
                "Chamfer
Click a corner where two lines meet to cut it; the distance is in the tool box"
            }
            Tool::Trim => {
                "Trim
Click the piece of a curve to remove, between the curves that cross it"
            }
            Tool::Extend => {
                "Extend
Click near the end of a line or arc to run it on to the next curve"
            }
            Tool::Split => {
                "Split
Click a curve to split it where other curves cross it; Split All splits every crossing"
            }
        }
    }

    fn hint(self, clicks: usize) -> &'static str {
        match (self, clicks) {
            (Tool::Select, _) => {
                "Click to select (Shift adds) · drag a point to move it · drag on empty space to box select · Del deletes"
            }
            (Tool::Line, 0) => "Click the start of the line",
            (Tool::Line, _) => {
                "Click the next point · click the first point to close · Esc or right-click ends"
            }
            (Tool::Rectangle, 0) => "Click the first corner",
            (Tool::Rectangle, _) => "Click the opposite corner",
            (Tool::CentreRectangle, 0) => "Click the centre of the rectangle",
            (Tool::CentreRectangle, _) => "Click a corner",
            (Tool::Circle, 0) => "Click the centre",
            (Tool::Circle, _) => "Click to set the radius",
            (Tool::Arc, 0) => "Click the centre",
            (Tool::Arc, 1) => "Click the start (sets the radius)",
            (Tool::Arc, _) => "Click the end (counter-clockwise from the start)",
            (Tool::ThreePointArc, 0) => "Click the start of the arc",
            (Tool::ThreePointArc, 1) => "Click the end of the arc",
            (Tool::ThreePointArc, _) => "Click a point the arc passes through",
            (Tool::TangentArc, 0) => "Click the free end of a line or arc",
            (Tool::TangentArc, _) => "Click where the arc ends · Esc cancels",
            (Tool::Offset, 0) => "Click a line, arc or circle: its chain is offset",
            (Tool::Offset, _) => "Click the side to offset toward · Esc cancels",
            (Tool::Mirror, _) => "Click the mirror line (select the geometry first) · Esc cancels",
            (Tool::RectangularPattern, _) => {
                "Set columns, rows and spacing, then Apply (select the geometry first)"
            }
            (Tool::CircularPattern, _) => {
                "Click the centre of the pattern (select the geometry first) · Esc cancels"
            }
            (Tool::Fillet, _) => "Click a corner to round it · Esc cancels",
            (Tool::Chamfer, _) => "Click a corner to cut it · Esc cancels",
            (Tool::Trim, _) => "Click the piece to remove · Esc cancels",
            (Tool::Extend, _) => "Click near the end to extend · Esc cancels",
            (Tool::Split, _) => "Click a curve to split it where others cross it · Esc cancels",
        }
    }

    /// Tools that act on existing geometry instead of placing points.
    fn edits(self) -> bool {
        matches!(
            self,
            Tool::Offset
                | Tool::Mirror
                | Tool::RectangularPattern
                | Tool::CircularPattern
                | Tool::Fillet
                | Tool::Chamfer
                | Tool::Trim
                | Tool::Extend
                | Tool::Split
        )
    }
}

/// A placed click: where it is, and the existing point it snapped to.
#[derive(Debug, Clone, Copy)]
struct Click {
    at: Point2,
    point: Option<u32>,
}

/// The values the editing tools use, as typed in the tool box.
pub struct ToolOptions {
    pub radius: String,
    pub chamfer: String,
    pub offset: String,
    pub offset_round: bool,
    pub columns: String,
    pub rows: String,
    pub dx: String,
    pub dy: String,
    pub count: String,
    pub angle: String,
}

impl Default for ToolOptions {
    fn default() -> Self {
        Self {
            radius: "5".into(),
            chamfer: "2".into(),
            offset: "2".into(),
            offset_round: false,
            columns: "3".into(),
            rows: "1".into(),
            dx: "20".into(),
            dy: "20".into(),
            count: "6".into(),
            angle: "360".into(),
        }
    }
}

#[derive(Default)]
pub struct SketchUi {
    pub tool: Option<Tool>,
    clicks: Vec<Click>,
    /// The first point of the current line chain, to close the loop on it.
    chain_start: Option<u32>,
    hover: Option<Point2>,
    /// The hovered pixel and viewport size, for previews that pick geometry.
    hover_px: Option<(f64, f64, f64, f64)>,
    /// The curve the Offset tool picked with its first click.
    picked_curve: Option<u32>,
    pub options: ToolOptions,
    dragging: Option<u32>,
    /// A dimension being edited: its constraint id, kind and the text typed so far.
    editing: Option<(u64, String, String)>,
    /// Remembered status line for the active sketch.
    pub status: Option<String>,
    /// The active sketch's constraint state, for the status bar's coloured dot.
    pub state: Option<SketchState>,
}

/// How constrained the active sketch is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SketchState {
    Full,
    Over,
    /// Under-constrained, with this many degrees of freedom.
    Free(u64),
    Unsolved,
}

impl SketchUi {
    pub fn tool(&self) -> Tool {
        self.tool.unwrap_or(Tool::Select)
    }

    pub(crate) fn reset_tool(&mut self) {
        self.clicks.clear();
        self.chain_start = None;
        self.picked_curve = None;
    }
}

/// The geometric constraints: command kind, name, icon and what it does.
const CONSTRAINTS: [(&str, &str, Icon, &str); 10] = [
    (
        "horizontal",
        "Horizontal",
        Icon::ConstraintHorizontal,
        "Level a line, or line up two points left to right",
    ),
    (
        "vertical",
        "Vertical",
        Icon::ConstraintVertical,
        "Stand a line upright, or line up two points top to bottom",
    ),
    (
        "coincident",
        "Coincident",
        Icon::ConstraintCoincident,
        "Join two points into one place",
    ),
    (
        "point_on",
        "Point on Curve",
        Icon::ConstraintPointOn,
        "Keep a point on a line, circle or arc",
    ),
    (
        "midpoint",
        "Midpoint",
        Icon::ConstraintMidpoint,
        "Keep a point halfway along a line",
    ),
    (
        "parallel",
        "Parallel",
        Icon::ConstraintParallel,
        "Keep two lines pointing the same way",
    ),
    (
        "perpendicular",
        "Perpendicular",
        Icon::ConstraintPerpendicular,
        "Keep two lines at a right angle",
    ),
    (
        "tangent",
        "Tangent",
        Icon::ConstraintTangent,
        "Make a line or curve touch a circle or arc smoothly",
    ),
    (
        "equal",
        "Equal",
        Icon::ConstraintEqual,
        "Give two lines the same length, or two circles the same radius",
    ),
    ("fix", "Fix", Icon::ConstraintFix, "Pin a point where it is"),
];

/// Plane names for the Sketch menu, with the view that looks straight at each.
pub const PLANES: [(&str, &str, &str); 3] = [
    ("xy", "Top plane (XY)", "top"),
    ("xz", "Front plane (XZ)", "front"),
    ("yz", "Right plane (YZ)", "right"),
];

fn as_id(v: &Value) -> Option<u32> {
    v.as_u64().and_then(|n| u32::try_from(n).ok())
}

impl ChamferaApp {
    /// Runs a sketch command and refreshes the sketch status line.
    pub(crate) fn run_sketch(&mut self, id: &str, params: Value) -> Option<Value> {
        let r = self.run(id, params);
        self.refresh_sketch_status();
        r
    }

    pub(crate) fn refresh_sketch_status(&mut self) {
        let (status, state) = match self.session.execute("sketch.get", &json!({})) {
            Ok(g) => {
                let name = g["name"].as_str().unwrap_or("Sketch").to_owned();
                let (kind, state) = match (g["status"].as_str(), g["dof"].as_u64()) {
                    (Some("FullyConstrained"), _) => {
                        (SketchState::Full, "fully constrained".to_owned())
                    }
                    (Some("OverConstrained"), _) => {
                        (SketchState::Over, "over-constrained".to_owned())
                    }
                    (_, Some(1)) => (SketchState::Free(1), "1 degree of freedom".to_owned()),
                    (_, Some(n)) => (SketchState::Free(n), format!("{n} degrees of freedom")),
                    _ => (SketchState::Unsolved, "not solved".to_owned()),
                };
                let regions = g["profiles"].as_array().map_or(0, Vec::len);
                (
                    Some(format!("{name} · {state} · {regions} closed profiles")),
                    Some(kind),
                )
            }
            Err(_) => (None, None),
        };
        self.sketch.status = status;
        self.sketch.state = state;
    }

    pub(crate) fn create_sketch(&mut self, plane: &str, view: &str) {
        if self
            .run_sketch("sketch.create", json!({ "plane": plane }))
            .is_some()
        {
            self.run("view.standard", json!({ "view": view }));
            self.sketch.tool = Some(Tool::Line);
            self.sketch.reset_tool();
        }
    }

    pub(crate) fn finish_sketch(&mut self) {
        self.run("sketch.finish", json!({}));
        self.sketch = super::sketch_ui::SketchUi::default();
    }

    /// The SKETCH ribbon, shown while a sketch is being edited: SELECT, CREATE,
    /// CONSTRAINTS, MODIFY and FINISH SKETCH groups.
    pub(crate) fn sketch_toolbar(&mut self, ui: &mut egui::Ui) {
        let look = self.look();
        let current = self.sketch.tool();
        let mut pick: Option<Tool> = None;
        let (_, _) = widgets::ribbon_group(ui, look, "SELECT", false, |ui| {
            if widgets::tool_button(
                ui,
                look,
                Icon::Select,
                "Select",
                true,
                current == Tool::Select,
                false,
            )
            .on_hover_text(crate::shortcuts::tip(
                Tool::Select.tooltip(),
                crate::shortcuts::Action::SketchTool(Tool::Select),
            ))
            .clicked()
            {
                pick = Some(Tool::Select);
            }
        });
        widgets::ribbon_divider(ui, look);
        let (_, menu) = widgets::ribbon_group(ui, look, "CREATE", true, |ui| {
            for tool in CREATE_SHOWN {
                if widgets::tool_button(
                    ui,
                    look,
                    tool.icon(),
                    tool.name(),
                    true,
                    current == tool,
                    false,
                )
                .on_hover_text(crate::shortcuts::tip(
                    tool.tooltip(),
                    crate::shortcuts::Action::SketchTool(tool),
                ))
                .clicked()
                {
                    pick = Some(tool);
                }
            }
        });
        egui::Popup::menu(&menu).show(|ui| {
            for tool in CREATE_ALL {
                if menu_item(ui, look, tool.icon(), tool.name(), true).clicked() {
                    ui.close();
                    pick = Some(tool);
                }
            }
        });
        if let Some(tool) = pick {
            self.sketch.tool = Some(tool);
            self.sketch.reset_tool();
        }
        let look = self.look();
        widgets::ribbon_divider(ui, look);

        let has_selection = !self.session.sketch_selection.is_empty();
        let mut constrain: Option<&str> = None;
        let mut dimension = false;
        let (_, menu) = widgets::ribbon_group(ui, look, "CONSTRAINTS", true, |ui| {
            if widgets::tool_button(
                ui,
                look,
                Icon::Dimension,
                "Dimension",
                has_selection,
                false,
                false,
            )
            .on_hover_text(crate::shortcuts::tip(
                &format!(
                    "{}\nSelect a line, a circle, an arc or two points, then click to give it a size",
                    label("sketch.dimension_selection")
                ),
                crate::shortcuts::Action::Dimension,
            ))
            .clicked()
            {
                dimension = true;
            }
            ui.add_space(widgets::S2);
            // The geometric constraints: a compact grid of two rows.
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(widgets::S1, widgets::S1);
                ui.add_space(1.0);
                for row in CONSTRAINTS.chunks(5) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = widgets::S1;
                        for (kind, text, icon, help) in row {
                            if widgets::icon_button(
                                ui,
                                look,
                                *icon,
                                26.0,
                                18.0,
                                has_selection,
                                false,
                            )
                            .on_hover_text(format!("{text}\n{help}"))
                            .clicked()
                            {
                                constrain = Some(kind);
                            }
                        }
                    });
                }
            });
        });
        egui::Popup::menu(&menu).show(|ui| {
            if menu_item(
                ui,
                look,
                Icon::Dimension,
                label("sketch.dimension_selection"),
                has_selection,
            )
            .clicked()
            {
                ui.close();
                dimension = true;
            }
            ui.separator();
            for (kind, text, icon, _) in CONSTRAINTS {
                if menu_item(ui, look, icon, text, has_selection).clicked() {
                    ui.close();
                    constrain = Some(kind);
                }
            }
            if !has_selection {
                ui.separator();
                widgets::caption(ui, look, "Select sketch geometry first");
            }
        });
        if let Some(kind) = constrain {
            self.run_sketch("sketch.constrain_selection", json!({ "kind": kind }));
        }
        if dimension {
            self.dimension_selected();
        }
        let look = self.look();
        widgets::ribbon_divider(ui, look);
        let mut construction = false;
        let mut remove = false;
        let mut split_all = false;
        let mut pick: Option<Tool> = None;
        let (_, menu) = widgets::ribbon_group(ui, look, "MODIFY", true, |ui| {
            for tool in MODIFY_TOOLS {
                if widgets::tool_button(
                    ui,
                    look,
                    tool.icon(),
                    tool.name(),
                    true,
                    current == tool,
                    false,
                )
                .on_hover_text(crate::shortcuts::tip(
                    tool.tooltip(),
                    crate::shortcuts::Action::SketchTool(tool),
                ))
                .clicked()
                {
                    pick = Some(tool);
                }
            }
            if widgets::tool_button(
                ui,
                look,
                Icon::Construction,
                label("sketch.construction"),
                has_selection,
                false,
                false,
            )
            .on_hover_text(crate::shortcuts::tip(
                "Construction\nTurn the selected curves into reference geometry that profiles ignore, or back",
                crate::shortcuts::Action::Construction,
            ))
            .clicked()
            {
                construction = true;
            }
            if widgets::tool_button(
                ui,
                look,
                Icon::Delete,
                label("sketch.remove"),
                has_selection,
                false,
                false,
            )
            .on_hover_text(crate::shortcuts::tip(
                "Delete\nRemove the selected points and curves",
                crate::shortcuts::Action::Delete,
            ))
            .clicked()
            {
                remove = true;
            }
        });
        egui::Popup::menu(&menu).show(|ui| {
            for tool in MODIFY_TOOLS {
                if menu_item(ui, look, tool.icon(), tool.name(), true).clicked() {
                    ui.close();
                    pick = Some(tool);
                }
            }
            if menu_item(ui, look, Icon::Split, "Split All Crossings", true)
                .on_hover_text("Split every curve where another crosses it, so overlapping shapes give their regions")
                .clicked()
            {
                ui.close();
                split_all = true;
            }
            ui.separator();
            if menu_item(ui, look, Icon::Construction, label("sketch.construction"), has_selection)
                .clicked()
            {
                ui.close();
                construction = true;
            }
            if menu_item(ui, look, Icon::Delete, label("sketch.remove"), has_selection).clicked() {
                ui.close();
                remove = true;
            }
        });
        if let Some(tool) = pick {
            self.sketch.tool = Some(tool);
            self.sketch.reset_tool();
        }
        if split_all {
            self.run_sketch("sketch.split", json!({}));
        }
        if construction {
            self.toggle_construction();
        }
        if remove {
            self.run_sketch("sketch.remove", json!({}));
        }
        let look = self.look();
        widgets::ribbon_divider(ui, look);
        let mut finish = false;
        widgets::ribbon_group(ui, look, "FINISH SKETCH", false, |ui| {
            if widgets::tool_button(
                ui,
                look,
                Icon::SketchFinish,
                label("sketch.finish"),
                true,
                false,
                false,
            )
            .on_hover_text("Finish Sketch\nLeave the sketch and go back to the solid tools")
            .clicked()
            {
                finish = true;
            }
        });
        if finish {
            self.finish_sketch();
        }
    }

    /// Dimensions the selection at its current size and starts editing the value, as the
    /// Dimension button does.
    pub(crate) fn dimension_selected(&mut self) {
        if let Some(r) = self.run_sketch("sketch.dimension_selection", json!({}))
            && let (Some(id), Some(v), Some(kind)) = (
                r["constraint"].as_u64(),
                r["value"].as_f64(),
                r["kind"].as_str(),
            )
        {
            self.sketch.editing = Some((id, kind.to_owned(), display_value(kind, v)));
        }
    }

    pub(crate) fn toggle_construction(&mut self) {
        let curves: Vec<u32> = self
            .session
            .sketch_selection
            .iter()
            .filter_map(|i| match i {
                SketchItem::Curve(id) => Some(*id),
                SketchItem::Point(_) => None,
            })
            .collect();
        let Some(f) = self.session.active_sketch() else {
            return;
        };
        let states: Vec<(u32, bool)> = curves
            .iter()
            .filter_map(|id| {
                f.sketch
                    .curve(chamfera_sketch::CurveId(*id))
                    .map(|c| (*id, !c.is_construction()))
            })
            .collect();
        for (id, value) in states {
            self.run_sketch(
                "sketch.construction",
                json!({ "curve": id, "construction": value }),
            );
        }
    }

    /// The point a click lands on: an existing sketch point under the cursor, else the
    /// plane point.
    fn click_at(&mut self, x: f64, y: f64, w: f64, h: f64) -> Option<Click> {
        let px = json!({ "x": x, "y": y, "width": w, "height": h });
        if let Ok(r) = self.session.execute("sketch.pick", &px)
            && let Some(id) = as_id(&r["item"]["point"])
            && let Some(at) = self
                .session
                .active_sketch()
                .and_then(|f| f.sketch.point(chamfera_sketch::PointId(id)))
        {
            return Some(Click {
                at,
                point: Some(id),
            });
        }
        let r = self.session.execute("sketch.plane_point", &px).ok()?;
        Some(Click {
            at: Point2::new(r["x"].as_f64()?, r["y"].as_f64()?),
            point: None,
        })
    }

    /// Mouse input in sketch mode. Returns true when the event was used (so the viewport
    /// does not also treat it as a body pick).
    pub(crate) fn sketch_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: Rect,
    ) -> bool {
        let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
        let local = |p: Pos2| (f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y));
        let (shift, esc, delete) = ui.input(|i| {
            (
                i.modifiers.shift,
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
            )
        });
        let typing = ui.ctx().egui_wants_keyboard_input();
        if esc && !typing {
            if self.sketch.clicks.is_empty() {
                self.sketch.tool = Some(Tool::Select);
                self.run_sketch("sketch.select_clear", json!({}));
            }
            self.sketch.reset_tool();
        }
        if delete && !typing && !self.session.sketch_selection.is_empty() {
            self.run_sketch("sketch.remove", json!({}));
        }
        // Hover: where the next click would land (for previews).
        self.sketch.hover_px = response.hover_pos().map(|p| {
            let (x, y) = local(p);
            (x, y, w, h)
        });
        self.sketch.hover = response
            .hover_pos()
            .and_then(|p| {
                let (x, y) = local(p);
                self.click_at(x, y, w, h)
            })
            .map(|c| c.at);

        let tool = self.sketch.tool();
        // Dragging a point with the Select tool.
        if tool == Tool::Select {
            if response.drag_started_by(egui::PointerButton::Primary)
                && let Some(p) = response.interact_pointer_pos()
            {
                let (x, y) = local(p);
                let picked = self
                    .session
                    .execute(
                        "sketch.pick",
                        &json!({ "x": x, "y": y, "width": w, "height": h }),
                    )
                    .ok()
                    .and_then(|r| as_id(&r["item"]["point"]));
                self.sketch.dragging = picked;
                if picked.is_none() {
                    self.box_begin(ui, rect, true);
                }
            }
            if self.box_input(ui, response, rect) {
                return true;
            }
            if let Some(point) = self.sketch.dragging {
                if response.dragged_by(egui::PointerButton::Primary)
                    && let Some(at) = self.sketch.hover
                {
                    self.run_sketch(
                        "sketch.drag",
                        json!({ "point": point, "x": at.x, "y": at.y }),
                    );
                }
                if response.drag_stopped() {
                    self.sketch.dragging = None;
                }
                return true;
            }
        }
        if response.secondary_clicked() && !self.sketch.clicks.is_empty() {
            self.sketch.reset_tool();
            return true;
        }
        if !response.clicked() {
            return false;
        }
        let Some(pos) = response.interact_pointer_pos() else {
            return false;
        };
        let (x, y) = local(pos);
        self.sketch_click(x, y, w, h, shift);
        true
    }

    /// A left click at pixel `(x, y)` of a `w` x `h` viewport in sketch mode: selects
    /// with the Select tool, else places the next point of the current tool.
    pub(crate) fn sketch_click(&mut self, x: f64, y: f64, w: f64, h: f64, shift: bool) {
        let tool = self.sketch.tool();
        if tool == Tool::Select {
            self.run_sketch(
                "sketch.select",
                json!({ "x": x, "y": y, "width": w, "height": h, "add": shift }),
            );
            return;
        }
        let Some(click) = self.click_at(x, y, w, h) else {
            self.message = Some(("The sketch plane is edge-on: turn the view".into(), true));
            return;
        };
        if tool.edits() {
            self.edit_click(tool, (x, y, w, h), click);
            return;
        }
        self.sketch.clicks.push(click);
        self.place(tool);
    }

    /// Creates geometry once a tool has all its clicks.
    fn place(&mut self, tool: Tool) {
        let c = self.sketch.clicks.clone();
        match (tool, c.as_slice()) {
            (Tool::Line, [a, b]) => {
                if a.at.coincident(b.at) {
                    self.sketch.clicks.pop();
                    return;
                }
                let mut p = json!({
                    "x1": a.at.x, "y1": a.at.y, "x2": b.at.x, "y2": b.at.y, "snap": true,
                });
                if let Some(id) = a.point {
                    p["start"] = json!(id);
                }
                if let Some(id) = b.point {
                    p["end"] = json!(id);
                }
                match self.run_sketch("sketch.line", p) {
                    Some(r) => {
                        let start = as_id(&r["start"]);
                        let end = as_id(&r["end"]);
                        if self.sketch.chain_start.is_none() {
                            self.sketch.chain_start = start;
                        }
                        let closed = end.is_some() && end == self.sketch.chain_start;
                        self.sketch.clicks.clear();
                        if closed {
                            self.sketch.chain_start = None;
                        } else {
                            self.sketch.clicks.push(Click {
                                at: b.at,
                                point: end,
                            });
                        }
                    }
                    None => {
                        self.sketch.clicks.pop();
                    }
                }
            }
            (Tool::Rectangle, [a, b]) => {
                self.run_sketch(
                    "sketch.rectangle",
                    json!({ "x1": a.at.x, "y1": a.at.y, "x2": b.at.x, "y2": b.at.y }),
                );
                self.sketch.clicks.clear();
            }
            (Tool::Circle, [a, b]) => {
                let r = a.at.distance(b.at);
                self.run_sketch(
                    "sketch.circle",
                    json!({ "cx": a.at.x, "cy": a.at.y, "radius": r }),
                );
                self.sketch.clicks.clear();
            }
            (Tool::Arc, [c0, s, e]) => {
                self.run_sketch(
                    "sketch.arc",
                    json!({
                        "cx": c0.at.x, "cy": c0.at.y,
                        "x1": s.at.x, "y1": s.at.y, "x2": e.at.x, "y2": e.at.y,
                    }),
                );
                self.sketch.clicks.clear();
            }
            (Tool::CentreRectangle, [c0, k]) => {
                self.run_sketch(
                    "sketch.rectangle.centre",
                    json!({ "cx": c0.at.x, "cy": c0.at.y, "x": k.at.x, "y": k.at.y }),
                );
                self.sketch.clicks.clear();
            }
            (Tool::ThreePointArc, [a, b, m]) => {
                self.run_sketch(
                    "sketch.arc.three_point",
                    json!({
                        "x1": a.at.x, "y1": a.at.y, "x2": m.at.x, "y2": m.at.y,
                        "x3": b.at.x, "y3": b.at.y,
                    }),
                );
                self.sketch.clicks.clear();
            }
            (Tool::TangentArc, [p]) if p.point.is_none() => {
                self.message = Some((
                    "Tangent Arc: start on the free end of a line or arc".into(),
                    true,
                ));
                self.sketch.clicks.clear();
            }
            (Tool::TangentArc, [p, q]) => {
                let r = self.run_sketch(
                    "sketch.arc.tangent",
                    json!({ "point": p.point, "x": q.at.x, "y": q.at.y }),
                );
                self.sketch.clicks.clear();
                // Chain on from the new arc's end, like the line tool.
                if let Some(r) = r
                    && let Some(end) = self.arc_end_point(&r, q.at)
                {
                    self.sketch.clicks.push(Click {
                        at: q.at,
                        point: Some(end),
                    });
                }
            }
            _ => {}
        }
    }

    /// The point at `at` among the points a command created.
    fn arc_end_point(&self, reply: &Value, at: Point2) -> Option<u32> {
        let f = self.session.active_sketch()?;
        reply["created_points"]
            .as_array()?
            .iter()
            .filter_map(as_id)
            .find(|p| {
                f.sketch
                    .point(PointId(*p))
                    .is_some_and(|q| q.coincident(at))
            })
    }

    /// The preview overlay for the tool in progress, in world coordinates.
    pub(crate) fn sketch_preview(&self) -> Option<Overlay> {
        if self.sketch.tool() == Tool::Select {
            return None;
        }
        let f = self.session.active_sketch()?;
        let hover = self.sketch.hover?;
        let c = &self.sketch.clicks;
        let w = |p: Point2| f.plane.from_plane(p);
        let mut lines: Vec<[Point3; 2]> = Vec::new();
        let mut polylines: Vec<Vec<Point2>> = Vec::new();
        let ring = |centre: Point2, r: f64, a0: f64, sweep: f64| -> Vec<[Point3; 2]> {
            let n = 64;
            (0..n)
                .map(|k| {
                    let t0 = a0 + sweep * k as f64 / n as f64;
                    let t1 = a0 + sweep * (k + 1) as f64 / n as f64;
                    [
                        w(Point2::new(
                            centre.x + r * t0.cos(),
                            centre.y + r * t0.sin(),
                        )),
                        w(Point2::new(
                            centre.x + r * t1.cos(),
                            centre.y + r * t1.sin(),
                        )),
                    ]
                })
                .collect()
        };
        match (self.sketch.tool(), c.as_slice()) {
            (Tool::Line, [a]) => lines.push([w(a.at), w(hover)]),
            (Tool::Rectangle, [a]) => {
                let q = [
                    a.at,
                    Point2::new(hover.x, a.at.y),
                    hover,
                    Point2::new(a.at.x, hover.y),
                ];
                for k in 0..4 {
                    lines.push([w(q[k]), w(q[(k + 1) % 4])]);
                }
            }
            (Tool::Circle, [a]) => {
                lines.extend(ring(a.at, a.at.distance(hover), 0.0, std::f64::consts::TAU));
            }
            (Tool::Arc, [a]) => lines.push([w(a.at), w(hover)]),
            (Tool::Arc, [a, s]) => {
                let a0 = (s.at - a.at).angle();
                let mut sweep = (hover - a.at).angle() - a0;
                if sweep <= 0.0 {
                    sweep += std::f64::consts::TAU;
                }
                lines.extend(ring(a.at, a.at.distance(s.at), a0, sweep));
            }
            (Tool::CentreRectangle, [a]) => {
                let op = ToolOp::CentreRectangle {
                    centre: a.at,
                    corner: hover,
                };
                polylines.extend(self.op_polylines(&op));
            }
            (Tool::ThreePointArc, [a]) => lines.push([w(a.at), w(hover)]),
            (Tool::ThreePointArc, [a, b]) => {
                let op = ToolOp::ThreePointArc {
                    start: a.at,
                    through: hover,
                    end: b.at,
                };
                polylines.extend(self.op_polylines(&op));
            }
            (Tool::TangentArc, [p]) => {
                if let Some(from) = p.point {
                    let op = ToolOp::TangentArc {
                        from: PointId(from),
                        to: hover,
                    };
                    polylines.extend(self.op_polylines(&op));
                }
            }
            (tool, _) if tool.edits() => polylines.extend(self.edit_preview(tool, hover)),
            _ => {}
        }
        for poly in polylines {
            lines.extend(poly.windows(2).map(|s| [w(s[0]), w(s[1])]));
        }
        Some(Overlay {
            style: OverlayStyle::Preview,
            lines,
            points: vec![w(hover)],
            on_top: true,
        })
    }

    /// Projects a world point to the viewport rect.
    fn project(&self, rect: Rect, p: Point3) -> Option<Pos2> {
        let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
        let bounds = self.session.scene.bounds();
        let vp = mat_mul(
            &self
                .session
                .camera
                .projection_matrix(w / h, bounds.as_ref()),
            &self.session.camera.view_matrix(),
        );
        let n = transform_point(&vp, p)?;
        if !(0.0..=1.0).contains(&n[2]) {
            return None;
        }
        Some(Pos2::new(
            rect.min.x + ((n[0] + 1.0) * 0.5 * w) as f32,
            rect.min.y + ((1.0 - n[1]) * 0.5 * h) as f32,
        ))
    }

    /// Dimension labels over the viewport; click one to edit its value.
    pub(crate) fn dimension_labels(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let labels = self.session.dimension_labels();
        let font = egui::FontId::proportional(12.5);
        let text_c = color(self.theme.text);
        let expr_c = color(self.theme.accent);
        let bg = color(self.theme.panel).gamma_multiply(0.96);
        let editing_id = self.sketch.editing.as_ref().map(|e| e.0);
        for l in labels {
            let Some(pos) = self.project(rect, l.anchor) else {
                continue;
            };
            if Some(u64::from(l.constraint)) == editing_id {
                continue;
            }
            let expression = self
                .session
                .active_sketch()
                .and_then(|f| f.expressions.get(&l.constraint).cloned());
            // The value, and the expression it follows in the accent colour.
            let mut job = egui::text::LayoutJob::default();
            let format = |c| egui::TextFormat::simple(font.clone(), c);
            job.append(&display_value(&l.kind, l.value), 0.0, format(text_c));
            if let Some(e) = &expression {
                job.append(&format!("= {e}"), 6.0, format(expr_c));
            }
            let galley = ui.painter().layout_job(job);
            let r = Rect::from_center_size(pos, galley.size() + Vec2::new(12.0, 6.0));
            let id = ui.scope_id().with(("dimension", l.constraint));
            let resp = ui
                .interact(r, id, egui::Sense::click())
                .on_hover_text("Click to type a value, a parameter or a formula");
            let border = if resp.hovered() {
                egui::Stroke::new(1.5, color(self.theme.accent))
            } else {
                egui::Stroke::new(1.0, color(self.theme.border))
            };
            let radius = egui::CornerRadius::same(widgets::R_SMALL);
            ui.painter().rect_filled(r, radius, bg);
            ui.painter()
                .rect_stroke(r, radius, border, egui::StrokeKind::Inside);
            ui.painter()
                .galley(r.center() - galley.size() * 0.5, galley, text_c);
            if resp.clicked() {
                let start = expression.unwrap_or_else(|| display_value(&l.kind, l.value));
                self.sketch.editing = Some((u64::from(l.constraint), l.kind.clone(), start));
            }
        }
        self.dimension_editor(ui, rect);
        self.sketch_tool_box(ui, rect);
    }

    /// The inline value box for the dimension being edited. Enter applies, Esc cancels.
    fn dimension_editor(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let Some((id, kind, mut text)) = self.sketch.editing.take() else {
            return;
        };
        let anchor = self
            .session
            .dimension_labels()
            .into_iter()
            .find(|l| u64::from(l.constraint) == id)
            .and_then(|l| self.project(rect, l.anchor))
            .unwrap_or(rect.center());
        let mut keep = true;
        let area = egui::Area::new(ui.scope_id().with(("dimension editor", id)))
            .fixed_pos(anchor - Vec2::new(50.0, 12.0))
            .order(egui::Order::Foreground);
        area.show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .desired_width(70.0)
                            .id(ui.scope_id().with(("dimension value", id))),
                    );
                    edit.request_focus();
                    ui.label(if kind == "angle" { "°" } else { "mm" });
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    if esc {
                        keep = false;
                    } else if enter {
                        // A plain number sets the value; anything else (a parameter, a
                        // formula) becomes an expression the dimension follows.
                        let params = match parse_value(&kind, &text) {
                            Some(v) => json!({ "constraint": id, "value": v }),
                            None => json!({ "constraint": id, "expression": text.trim() }),
                        };
                        if self.run_sketch("sketch.dimension", params).is_some() {
                            keep = false;
                        }
                    }
                });
            });
        });
        if keep {
            self.sketch.editing = Some((id, kind, text));
        }
    }
}

/// A dimension value as the user reads it: mm with two decimals, angles in degrees.
fn display_value(kind: &str, v: f64) -> String {
    let text = if kind == "angle" {
        format!("{:.2}", v.to_degrees())
    } else {
        format!("{v:.2}")
    };
    let text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    match kind {
        "diameter" => format!("Ø{text}"),
        "radius" => format!("R{text}"),
        "angle" => format!("{text}°"),
        _ => text,
    }
}

/// Parses a typed dimension value (mm, or degrees for angles) to internal units.
fn parse_value(kind: &str, text: &str) -> Option<f64> {
    let cleaned: String = text
        .trim()
        .trim_start_matches(['Ø', 'ø', 'R', 'r'])
        .trim_end_matches(['°', 'm'])
        .trim()
        .replace(',', ".");
    let v: f64 = cleaned.parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    Some(if kind == "angle" { v.to_radians() } else { v })
}

// ----- the editing tools: trim, extend, split, fillet, chamfer, offset, mirror, patterns -----

/// A length typed in the tool box, in mm.
fn option_length(text: &str) -> Option<f64> {
    parse_value("length", text).filter(|v| *v > 0.0)
}

/// A whole number typed in the tool box.
fn option_count(text: &str) -> Option<u32> {
    text.trim().parse().ok()
}

impl ChamferaApp {
    /// The selected curves and points, without `skip`.
    fn selected_ids(&self, skip: Option<u32>) -> (Vec<u32>, Vec<u32>) {
        let mut curves = Vec::new();
        let mut points = Vec::new();
        for item in &self.session.sketch_selection {
            match item {
                SketchItem::Curve(c) if Some(*c) != skip => curves.push(*c),
                SketchItem::Point(p) => points.push(*p),
                SketchItem::Curve(_) => {}
            }
        }
        (curves, points)
    }

    /// What an editing tool would do at pixel `(x, y)` (sketch point `at`): the command,
    /// its parameters and the same operation for the preview. `None` when nothing there
    /// suits the tool.
    fn tool_op(
        &self,
        tool: Tool,
        px: (f64, f64, f64, f64),
        at: Point2,
    ) -> Option<(&'static str, Value, ToolOp)> {
        let (x, y, w, h) = px;
        let item = self.session.pick_sketch_item(x, y, w, h);
        let curve = match item {
            Some(SketchItem::Curve(c)) => Some(c),
            _ => None,
        };
        let point = match item {
            Some(SketchItem::Point(p)) => Some(p),
            _ => None,
        };
        let o = &self.sketch.options;
        match tool {
            Tool::Trim | Tool::Extend => {
                let c = curve?;
                let p = json!({ "curve": c, "x": at.x, "y": at.y });
                let op = if tool == Tool::Trim {
                    ToolOp::Trim {
                        curve: CurveId(c),
                        at,
                    }
                } else {
                    ToolOp::Extend {
                        curve: CurveId(c),
                        at,
                    }
                };
                let id = if tool == Tool::Trim {
                    "sketch.trim"
                } else {
                    "sketch.extend"
                };
                Some((id, p, op))
            }
            Tool::Split => {
                let c = curve?;
                Some((
                    "sketch.split",
                    json!({ "curve": c }),
                    ToolOp::Split {
                        curve: Some(CurveId(c)),
                    },
                ))
            }
            Tool::Fillet => {
                let corner = point?;
                let radius = option_length(&o.radius)?;
                Some((
                    "sketch.fillet",
                    json!({ "point": corner, "radius": radius }),
                    ToolOp::Fillet {
                        corner: PointId(corner),
                        radius,
                    },
                ))
            }
            Tool::Chamfer => {
                let corner = point?;
                let distance = option_length(&o.chamfer)?;
                Some((
                    "sketch.chamfer",
                    json!({ "point": corner, "distance": distance }),
                    ToolOp::Chamfer {
                        corner: PointId(corner),
                        distance,
                        distance2: None,
                    },
                ))
            }
            Tool::Offset => {
                let c = self.sketch.picked_curve?;
                let distance = option_length(&o.offset)?;
                let corners = if o.offset_round {
                    OffsetCorners::Round
                } else {
                    OffsetCorners::Sharp
                };
                Some((
                    "sketch.offset",
                    json!({
                        "curves": [c], "distance": distance, "x": at.x, "y": at.y,
                        "corners": if o.offset_round { "round" } else { "sharp" },
                    }),
                    ToolOp::Offset {
                        curves: vec![CurveId(c)],
                        distance,
                        side: Some(at),
                        corners,
                        dimension: true,
                    },
                ))
            }
            Tool::Mirror => {
                let axis = curve?;
                let (curves, points) = self.selected_ids(Some(axis));
                if curves.is_empty() && points.is_empty() {
                    return None;
                }
                Some((
                    "sketch.mirror",
                    json!({ "axis": axis, "curves": curves, "points": points }),
                    ToolOp::Mirror {
                        curves: curves.iter().map(|c| CurveId(*c)).collect(),
                        points: points.iter().map(|p| PointId(*p)).collect(),
                        axis: CurveId(axis),
                    },
                ))
            }
            Tool::CircularPattern => {
                let (curves, points) = self.selected_ids(None);
                if curves.is_empty() && points.is_empty() {
                    return None;
                }
                let count = option_count(&o.count)?;
                let angle = parse_value("angle", &o.angle)?;
                Some((
                    "sketch.pattern.circular",
                    json!({
                        "curves": curves, "points": points, "cx": at.x, "cy": at.y,
                        "count": count, "angle": angle,
                    }),
                    ToolOp::CircularPattern {
                        curves: curves.iter().map(|c| CurveId(*c)).collect(),
                        points: points.iter().map(|p| PointId(*p)).collect(),
                        centre: at,
                        count,
                        angle,
                    },
                ))
            }
            _ => None,
        }
    }

    /// The rectangular pattern of the selection, from the tool box.
    fn rectangular_pattern_op(&self) -> Option<(Value, ToolOp)> {
        let (curves, points) = self.selected_ids(None);
        if curves.is_empty() && points.is_empty() {
            return None;
        }
        let o = &self.sketch.options;
        let columns = option_count(&o.columns)?;
        let rows = option_count(&o.rows)?;
        let dx = parse_value("length", &o.dx)?;
        let dy = parse_value("length", &o.dy)?;
        Some((
            json!({
                "curves": curves, "points": points, "columns": columns, "rows": rows,
                "dx": dx, "dy": dy,
            }),
            ToolOp::RectangularPattern {
                curves: curves.iter().map(|c| CurveId(*c)).collect(),
                points: points.iter().map(|p| PointId(*p)).collect(),
                columns,
                rows,
                dx,
                dy,
            },
        ))
    }

    /// A click of an editing tool.
    fn edit_click(&mut self, tool: Tool, px: (f64, f64, f64, f64), click: Click) {
        if tool == Tool::Offset && self.sketch.picked_curve.is_none() {
            let (x, y, w, h) = px;
            match self.session.pick_sketch_item(x, y, w, h) {
                Some(SketchItem::Curve(c)) => {
                    self.sketch.picked_curve = Some(c);
                    self.sketch.clicks.push(click);
                }
                _ => {
                    self.message = Some(("Offset: click a line, arc or circle".into(), true));
                }
            }
            return;
        }
        if tool == Tool::RectangularPattern {
            return;
        }
        let Some((id, params, _)) = self.tool_op(tool, px, click.at) else {
            let what = match tool {
                Tool::Fillet | Tool::Chamfer => "click a corner point where two lines meet",
                Tool::Mirror => "select the geometry, then click a line to mirror it in",
                Tool::CircularPattern => "select the geometry first",
                Tool::Offset => "type a distance above 0",
                _ => "click a curve",
            };
            self.message = Some((format!("{}: {what}", tool.name()), true));
            return;
        };
        if self.run_sketch(id, params).is_some() {
            self.message = None;
            self.sketch.reset_tool();
        }
    }

    /// The preview of an editing tool under the cursor, in sketch coordinates.
    fn edit_preview(&self, tool: Tool, hover: Point2) -> Vec<Vec<Point2>> {
        let Some(f) = self.session.active_sketch() else {
            return Vec::new();
        };
        let op = if tool == Tool::RectangularPattern {
            self.rectangular_pattern_op().map(|(_, op)| op)
        } else {
            match self.sketch.hover_px {
                Some(px) => self.tool_op(tool, px, hover).map(|(_, _, op)| op),
                None => None,
            }
        };
        let Some(op) = op else {
            // Offset: show the picked chain's curve while the side is chosen.
            if let Some(c) = self.sketch.picked_curve
                && let Some(curve) = f.sketch.curve(CurveId(c))
            {
                return vec![curve_polyline(&f.sketch, curve)];
            }
            return Vec::new();
        };
        match op {
            ToolOp::Trim { curve, at } => f.sketch.trim_preview(curve, at).into_iter().collect(),
            ToolOp::Extend { curve, at } => {
                f.sketch.extend_preview(curve, at).into_iter().collect()
            }
            op => self.op_polylines(&op),
        }
    }

    /// The curves a tool would make or change, as polylines, for its preview.
    fn op_polylines(&self, op: &ToolOp) -> Vec<Vec<Point2>> {
        let Some(f) = self.session.active_sketch() else {
            return Vec::new();
        };
        let Some((after, report)) = f.sketch.preview_tool(op) else {
            return Vec::new();
        };
        report
            .created_curves
            .iter()
            .chain(&report.changed_curves)
            .filter_map(|c| after.curve(*c))
            .filter(|c| !c.is_construction())
            .map(|c| curve_polyline(&after, c))
            .collect()
    }

    /// The tool box for tools that take values (fillet radius, offset distance, pattern
    /// counts), at the viewport's top left.
    fn sketch_tool_box(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let tool = self.sketch.tool();
        if !matches!(
            tool,
            Tool::Fillet
                | Tool::Chamfer
                | Tool::Offset
                | Tool::Mirror
                | Tool::RectangularPattern
                | Tool::CircularPattern
                | Tool::Split
        ) {
            return;
        }
        let look = widgets::Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let mut apply = false;
        let mut split_all = false;
        let selected = self.session.sketch_selection.len();
        let field = |ui: &mut egui::Ui, name: &str, text: &mut String, unit: &str| {
            ui.horizontal(|ui| {
                widgets::field_label(ui, look, name);
                ui.add(egui::TextEdit::singleline(text).desired_width(64.0));
                ui.label(egui::RichText::new(unit).color(look.c(|t| t.text_muted)));
            });
        };
        let options = &mut self.sketch.options;
        egui::Area::new(ui.scope_id().with("sketch tool box"))
            .fixed_pos(rect.min + Vec2::new(widgets::S5, widgets::S5))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                widgets::dialog_frame(look)
                    .inner_margin(egui::Margin::same(widgets::S4 as i8))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = widgets::S2;
                        ui.horizontal(|ui| {
                            let (r, _) =
                                ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
                            look.icon(ui, tool.icon(), r, true);
                            ui.label(
                                egui::RichText::new(tool.name())
                                    .size(widgets::T_TITLE)
                                    .color(look.c(|t| t.text)),
                            );
                        });
                        match tool {
                            Tool::Fillet => field(ui, "Radius", &mut options.radius, "mm"),
                            Tool::Chamfer => field(ui, "Distance", &mut options.chamfer, "mm"),
                            Tool::Offset => {
                                field(ui, "Distance", &mut options.offset, "mm");
                                ui.checkbox(&mut options.offset_round, "Round outside corners");
                            }
                            Tool::Mirror => {
                                widgets::caption(
                                    ui,
                                    look,
                                    &format!("{selected} selected · click the mirror line"),
                                );
                            }
                            Tool::CircularPattern => {
                                field(ui, "Count", &mut options.count, "");
                                field(ui, "Angle", &mut options.angle, "°");
                                widgets::caption(
                                    ui,
                                    look,
                                    &format!("{selected} selected · click the centre"),
                                );
                            }
                            Tool::RectangularPattern => {
                                field(ui, "Columns", &mut options.columns, "");
                                field(ui, "Spacing X", &mut options.dx, "mm");
                                field(ui, "Rows", &mut options.rows, "");
                                field(ui, "Spacing Y", &mut options.dy, "mm");
                                ui.horizontal(|ui| {
                                    widgets::caption(ui, look, &format!("{selected} selected"));
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            apply = widgets::primary_button(
                                                ui,
                                                look,
                                                "Apply",
                                                selected > 0,
                                            )
                                            .clicked();
                                        },
                                    );
                                });
                            }
                            Tool::Split => {
                                split_all =
                                    widgets::secondary_button(ui, look, "Split All Crossings")
                                        .clicked();
                            }
                            _ => {}
                        }
                    });
            });
        if apply {
            match self.rectangular_pattern_op() {
                Some((params, _)) => {
                    self.run_sketch("sketch.pattern.rectangular", params);
                }
                None => {
                    self.message = Some((
                        "Rectangular Pattern: select geometry and type whole numbers".into(),
                        true,
                    ));
                }
            }
        }
        if split_all {
            self.run_sketch("sketch.split", json!({}));
        }
    }
}

pub(crate) fn tool_hint(app: &ChamferaApp) -> &'static str {
    app.sketch.tool().hint(app.sketch.clicks.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A top view of the XY plane, 800 x 600, centred on (40, 25) at a distance that makes
    /// the pixel scale easy: `px(x, y)` is the pixel of sketch point (x, y).
    fn app_on_top_plane() -> (ChamferaApp, impl Fn(f64, f64) -> (f64, f64)) {
        let mut app = ChamferaApp::headless();
        app.create_sketch("xy", "top");
        app.session.camera.target = chamfera_geom::Point3::new(40.0, 25.0, 0.0);
        app.session.camera.projection = chamfera_render::Projection::Orthographic;
        let cam = app.session.camera;
        // Orthographic: the view is 2 * half_height tall over 600 px.
        let per_mm = 600.0 / (2.0 * cam.half_height());
        let px = move |x: f64, y: f64| (400.0 + (x - 40.0) * per_mm, 300.0 - (y - 25.0) * per_mm);
        (app, px)
    }

    fn click(app: &mut ChamferaApp, p: (f64, f64)) {
        app.sketch_click(p.0, p.1, 800.0, 600.0, false);
    }

    #[test]
    fn four_line_clicks_ending_on_the_first_point_close_a_profile() {
        let (mut app, px) = app_on_top_plane();
        app.sketch.tool = Some(Tool::Line);
        for (x, y) in [
            (0.0, 0.0),
            (80.0, 0.0),
            (80.0, 50.0),
            (0.0, 50.0),
            (0.0, 0.0),
        ] {
            click(&mut app, px(x, y));
        }
        let g = app.session.execute("sketch.get", &json!({})).unwrap();
        assert_eq!(g["curves"].as_array().unwrap().len(), 4);
        // Shared corners: 4 points, not 8, and the chain ended on closing.
        assert_eq!(g["points"].as_array().unwrap().len(), 4);
        assert!(app.sketch.clicks.is_empty());
        let area = g["profiles"][0]["area"].as_f64().unwrap();
        // Clicks land on pixel centres of an orthographic view, so allow 0.5 px of error
        // per side: the rectangle is 80 x 50 to within that.
        assert!((area - 4000.0).abs() < 4000.0 * 0.02, "{area}");
        // Snapping made the four sides horizontal and vertical.
        let snaps = g["constraints"].as_array().unwrap().len();
        assert_eq!(snaps, 4);
    }

    #[test]
    fn rectangle_and_circle_tools_take_two_clicks_each() {
        let (mut app, px) = app_on_top_plane();
        app.sketch.tool = Some(Tool::Rectangle);
        click(&mut app, px(0.0, 0.0));
        click(&mut app, px(80.0, 50.0));
        app.sketch.tool = Some(Tool::Circle);
        click(&mut app, px(20.0, 25.0));
        click(&mut app, px(28.0, 25.0));
        let g = app.session.execute("sketch.get", &json!({})).unwrap();
        let profiles = g["profiles"].as_array().unwrap();
        assert_eq!(profiles.len(), 2, "plate with a hole, and the disc");
        assert!(profiles.iter().any(|r| r["holes"] == 1));
    }

    #[test]
    fn clicking_selects_and_dimension_edits_resize() {
        let (mut app, px) = app_on_top_plane();
        app.sketch.tool = Some(Tool::Rectangle);
        click(&mut app, px(0.0, 0.0));
        click(&mut app, px(80.0, 50.0));
        app.sketch.tool = Some(Tool::Select);
        // The bottom edge, away from its corners.
        click(&mut app, px(40.0, 0.0));
        assert_eq!(app.session.sketch_selection.len(), 1);
        let r = app
            .run_sketch("sketch.dimension_selection", json!({}))
            .unwrap();
        let id = r["constraint"].as_u64().unwrap();
        app.run_sketch(
            "sketch.dimension",
            json!({ "constraint": id, "value": 100 }),
        );
        let g = app.session.execute("sketch.get", &json!({})).unwrap();
        let xs: Vec<f64> = g["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["x"].as_f64().unwrap())
            .collect();
        let width = xs.iter().cloned().fold(f64::MIN, f64::max)
            - xs.iter().cloned().fold(f64::MAX, f64::min);
        assert!((width - 100.0).abs() < 1e-6, "{width}");
        assert!(
            app.sketch
                .status
                .as_deref()
                .unwrap_or("")
                .contains("degrees of freedom")
        );
    }

    #[test]
    fn dimension_values_read_and_parse_both_ways() {
        assert_eq!(display_value("length", 50.0), "50");
        assert_eq!(display_value("length", 12.345), "12.35");
        assert_eq!(display_value("diameter", 16.0), "Ø16");
        assert_eq!(display_value("radius", 8.5), "R8.5");
        assert_eq!(display_value("angle", std::f64::consts::FRAC_PI_2), "90°");
        assert_eq!(parse_value("length", " 42,5 mm"), Some(42.5));
        assert_eq!(parse_value("diameter", "Ø16"), Some(16.0));
        let a = parse_value("angle", "45°").unwrap();
        assert!((a - std::f64::consts::FRAC_PI_4).abs() < 1e-15);
        assert_eq!(parse_value("length", "abc"), None);
        assert_eq!(parse_value("length", "inf"), None);
    }

    fn areas(app: &mut ChamferaApp) -> Vec<f64> {
        let g = app.session.execute("sketch.get", &json!({})).unwrap();
        let mut a: Vec<f64> = g["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["area"].as_f64().unwrap())
            .collect();
        a.sort_by(f64::total_cmp);
        a
    }

    fn curve_count(app: &mut ChamferaApp) -> usize {
        let g = app.session.execute("sketch.get", &json!({})).unwrap();
        g["curves"].as_array().unwrap().len()
    }

    /// An exact 80 x 50 rectangle from (0, 0), drawn by command.
    fn plate(app: &mut ChamferaApp) {
        app.run_sketch(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 80, "y2": 50 }),
        )
        .unwrap();
    }

    #[test]
    fn trim_tool_clicks_remove_the_piece_under_the_cursor() {
        let (mut app, px) = app_on_top_plane();
        app.run_sketch(
            "sketch.line",
            json!({ "x1": 0, "y1": 25, "x2": 80, "y2": 25 }),
        )
        .unwrap();
        app.run_sketch(
            "sketch.line",
            json!({ "x1": 40, "y1": 0, "x2": 40, "y2": 50 }),
        )
        .unwrap();
        app.sketch.tool = Some(Tool::Trim);
        assert!(tool_hint(&app).contains("piece"));
        click(&mut app, px(60.0, 25.0));
        assert_eq!(curve_count(&mut app), 2);
        let f = app.session.active_sketch().unwrap();
        // The horizontal line now ends exactly at the crossing (40, 25).
        let ends: Vec<Point2> = f.sketch.points().map(|(_, p)| p).collect();
        assert!(
            ends.iter()
                .any(|p| p.distance(Point2::new(40.0, 25.0)) < 1e-9)
        );
        assert!(
            !ends
                .iter()
                .any(|p| p.distance(Point2::new(80.0, 25.0)) < 1e-9)
        );
        // Clicking empty space does nothing and says what to do.
        click(&mut app, px(10.0, 10.0));
        assert_eq!(curve_count(&mut app), 2);
        assert!(app.message.is_some());
    }

    #[test]
    fn fillet_tool_clicks_round_each_corner() {
        let (mut app, px) = app_on_top_plane();
        plate(&mut app);
        app.sketch.tool = Some(Tool::Fillet);
        app.sketch.options.radius = "5".into();
        for (x, y) in [(0.0, 0.0), (80.0, 0.0), (80.0, 50.0), (0.0, 50.0)] {
            click(&mut app, px(x, y));
        }
        // 80 x 50 less (4 - π) r² at r = 5.
        let a = areas(&mut app);
        let expected = 4000.0 - (4.0 - std::f64::consts::PI) * 25.0;
        assert!((a[0] - expected).abs() < 1e-6, "{a:?}");
        // A chamfer of 2 on a fresh plate's corner takes 2 · 2 / 2.
        let (mut app, px) = app_on_top_plane();
        plate(&mut app);
        app.sketch.tool = Some(Tool::Chamfer);
        click(&mut app, px(80.0, 50.0));
        let a = areas(&mut app);
        assert!((a[0] - 3998.0).abs() < 1e-6, "{a:?}");
    }

    #[test]
    fn offset_tool_takes_a_chain_then_a_side() {
        let (mut app, px) = app_on_top_plane();
        plate(&mut app);
        app.sketch.tool = Some(Tool::Offset);
        app.sketch.options.offset = "2".into();
        click(&mut app, px(40.0, 0.0));
        assert!(tool_hint(&app).contains("side"));
        // The preview follows the cursor toward the outside.
        app.sketch.hover = Some(Point2::new(40.0, -10.0));
        app.sketch.hover_px = Some((px(40.0, -10.0).0, px(40.0, -10.0).1, 800.0, 600.0));
        let preview = app.sketch_preview().unwrap();
        assert!(preview.lines.len() >= 4);
        click(&mut app, px(40.0, -10.0));
        // Sharp corners: an 84 x 54 rectangle around the 80 x 50.
        let a = areas(&mut app);
        assert!((a[0] - (84.0 * 54.0 - 4000.0)).abs() < 1e-6, "{a:?}");
        assert!(app.sketch.picked_curve.is_none());
    }

    #[test]
    fn mirror_and_circular_pattern_tools_copy_the_selection() {
        let (mut app, px) = app_on_top_plane();
        app.run_sketch(
            "sketch.line",
            json!({ "x1": 40, "y1": 0, "x2": 40, "y2": 50 }),
        )
        .unwrap();
        app.run_sketch("sketch.circle", json!({ "cx": 50, "cy": 25, "radius": 4 }))
            .unwrap();
        app.sketch.tool = Some(Tool::Select);
        click(&mut app, px(54.0, 25.0));
        assert_eq!(app.session.sketch_selection.len(), 1);
        app.sketch.tool = Some(Tool::Mirror);
        click(&mut app, px(40.0, 10.0));
        let f = app.session.active_sketch().unwrap();
        // The image of (50, 25) in x = 40 is (30, 25).
        assert!(
            f.sketch
                .points()
                .any(|(_, p)| p.distance(Point2::new(30.0, 25.0)) < 1e-9)
        );
        assert_eq!(curve_count(&mut app), 3);
        // Four around the line's midpoint (40, 25).
        app.sketch.tool = Some(Tool::Select);
        click(&mut app, px(54.0, 25.0));
        app.sketch.tool = Some(Tool::CircularPattern);
        app.sketch.options.count = "4".into();
        click(&mut app, px(40.0, 25.0));
        assert_eq!(curve_count(&mut app), 6);
    }

    #[test]
    fn centre_rectangle_and_three_point_arc_tools_take_their_clicks() {
        let (mut app, px) = app_on_top_plane();
        app.sketch.tool = Some(Tool::CentreRectangle);
        click(&mut app, px(40.0, 25.0));
        assert!(tool_hint(&app).contains("corner"));
        click(&mut app, px(60.0, 35.0));
        let a = areas(&mut app);
        // 40 x 20 to within half a pixel per side.
        assert!((a[0] - 800.0).abs() < 800.0 * 0.03, "{a:?}");
        app.sketch.tool = Some(Tool::ThreePointArc);
        click(&mut app, px(0.0, 0.0));
        click(&mut app, px(20.0, 0.0));
        click(&mut app, px(10.0, 10.0));
        let f = app.session.active_sketch().unwrap();
        assert!(
            f.sketch
                .curves()
                .any(|(_, c)| matches!(c, chamfera_sketch::Curve::Arc { .. }))
        );
        assert!(app.sketch.clicks.is_empty());
    }

    #[test]
    fn the_ribbon_lists_every_tool_with_an_icon_and_a_hint() {
        for tool in CREATE_ALL.into_iter().chain(MODIFY_TOOLS) {
            assert!(!tool.name().is_empty());
            assert!(tool.tooltip().starts_with(tool.name()), "{tool:?}");
            assert!(!tool.hint(0).is_empty() && !tool.hint(1).is_empty());
            let _ = tool.icon();
        }
        for tool in CREATE_SHOWN {
            assert!(CREATE_ALL.contains(&tool));
        }
    }
}
