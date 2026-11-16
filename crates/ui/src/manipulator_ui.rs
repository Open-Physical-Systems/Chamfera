//! Drag handles in the viewport (manipulators) for the tools that take a distance or a
//! placement: an arrow on the Extrude tool's profile, a size handle on the Fillet or
//! Chamfer tool's first edge, and a triad (three axis arrows, three plane squares and
//! three turning arcs) for Move and Move Component.
//!
//! A drag only changes the open dialog's fields, exactly as typing would: the live
//! preview follows them and OK runs the dialog's one command, so a whole drag is one undo
//! step and typing still works. The geometry (projection, ray-to-axis drags, turn
//! angles, the screen-constant size, hit tests, snapping steps) is in
//! `chamfera_render::manipulator`; this file draws the handles and feeds them the
//! pointer. Lengths snap to steps of the drawn grid (Shift: a tenth of that), turns to
//! 5 degrees (Shift: 1 degree).

use crate::widgets::{self, Look};
use crate::{ChamferaApp, color};
use chamfera_doc::ExtrudeDirection;
use chamfera_doc::expr::{self, Dim};
use chamfera_geom::{Aabb, Point3, Vec3};
use chamfera_render::manipulator::{
    self as m, ANGLE_STEP, Arrow, FINE_ANGLE_STEP, GRAB_PIXELS, HANDLE_PIXELS, Pixel, ScreenView,
    Triad, TriadPart,
};
use egui::{Color32, Pos2, Rect, Shape, Stroke, Vec2};
use serde_json::{Value, json};

/// Which open tool a handle edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    Extrude,
    Move,
    Component(u32),
    Blend,
}

/// The part of a handle the pointer is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    /// The arrow (or grip) of a one-value handle.
    Value,
    Triad(TriadPart),
}

/// A handle as it stands now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Handle {
    /// One value along a line: the grip sits at `origin + dir * (value * per_value)` and
    /// the arrow continues along `dir`. `stem` draws the line from `origin` to the grip
    /// (a fillet's radius measured into the solid).
    Value {
        origin: Point3,
        dir: Vec3,
        per_value: f64,
        value: f64,
        stem: bool,
    },
    /// A triad drawn at `centre`; `pivot` is where the bodies are before the move, which
    /// a turn's axis passes through.
    Triad { centre: Point3, pivot: Point3 },
}

impl Handle {
    fn grip(&self) -> Point3 {
        match *self {
            Handle::Value {
                origin,
                dir,
                per_value,
                value,
                ..
            } => origin + dir * (value * per_value),
            Handle::Triad { centre, .. } => centre,
        }
    }
}

/// A drag in progress: the handle and view as they were when it started (so the line
/// it follows does not move under it), the values then, and the turn so far.
#[derive(Debug, Clone, Copy)]
struct Drag {
    target: Target,
    part: Part,
    from: Pixel,
    last: Pixel,
    turned: f64,
    start: [f64; 3],
    view: ScreenView,
    handle: Handle,
}

/// The manipulators' state, kept in `SolidUi`.
#[derive(Default)]
pub struct ManipState {
    drag: Option<Drag>,
    hover: Option<Part>,
    /// `--manipulate PART=VALUE`: set the open handle as a drag would, on the first frame.
    pub demo: Option<String>,
    /// The part the demo set, drawn as if held.
    shown: Option<Part>,
    /// The move preview shown: the command and parameters.
    preview_for: Option<Value>,
}

/// `v` mm as a dialog field: up to 4 decimals, trailing zeros dropped.
fn length_text(v: f64) -> String {
    format!("{} mm", trimmed(v))
}

/// `a` radians as a dialog field in degrees.
fn angle_text(a: f64) -> String {
    format!("{} deg", trimmed(a.to_degrees()))
}

fn trimmed(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.to_owned()
    }
}

/// The parsed axis index of a plane-drag or turn part's two in-plane axes.
fn others(i: usize) -> [usize; 2] {
    [(i + 1) % 3, (i + 2) % 3]
}

/// Reads `--manipulate`: `value=15` (the arrow or size handle), `x=20` (an axis),
/// `xy=10,5` (a plane square: values along its two axes) or `turn-z=30` (degrees).
pub(crate) fn parse_demo(spec: &str) -> Result<(Part, [f64; 3]), String> {
    let usage = "--manipulate takes value=D, x=D, y=D, z=D, xy=D1,D2, yz=D1,D2, xz=D1,D2 \
                 or turn-x=DEG, turn-y=DEG, turn-z=DEG";
    let (part, values) = spec.split_once('=').ok_or(usage)?;
    let nums: Vec<f64> = values
        .split(',')
        .map(|v| v.trim().parse::<f64>().map_err(|_| usage.to_owned()))
        .collect::<Result<_, _>>()?;
    if nums.iter().any(|v| !v.is_finite() || v.abs() > 1e6) {
        return Err(usage.into());
    }
    let one = |i: usize| -> Result<[f64; 3], String> {
        match nums.as_slice() {
            [v] => {
                let mut d = [0.0; 3];
                if let Some(slot) = d.get_mut(i) {
                    *slot = *v;
                }
                Ok(d)
            }
            _ => Err(usage.into()),
        }
    };
    let axis = |c: &str| ["x", "y", "z"].iter().position(|a| *a == c);
    let part = part.trim().to_ascii_lowercase();
    if part == "value" {
        return Ok((Part::Value, one(0)?));
    }
    if let Some(i) = axis(&part) {
        return Ok((Part::Triad(TriadPart::Axis(i)), one(i)?));
    }
    if let Some(a) = part.strip_prefix("turn-").and_then(axis) {
        let mut d = one(0)?;
        d[0] = d[0].to_radians();
        return Ok((Part::Triad(TriadPart::Ring(a)), d));
    }
    // A plane by its two axes: xy is normal to z, yz to x, xz to y.
    let normal = match part.as_str() {
        "yz" | "zy" => 0,
        "xz" | "zx" => 1,
        "xy" | "yx" => 2,
        _ => return Err(usage.into()),
    };
    let [a, b] = others(normal);
    match nums.as_slice() {
        [u, v] => {
            let mut d = [0.0; 3];
            // The values come in x, y, z order of the two axes named.
            let (first, second) = if a < b { (a, b) } else { (b, a) };
            if let Some(s) = d.get_mut(first) {
                *s = *u;
            }
            if let Some(s) = d.get_mut(second) {
                *s = *v;
            }
            Ok((Part::Triad(TriadPart::Plane(normal)), d))
        }
        _ => Err(usage.into()),
    }
}

/// The dialog row naming where a turn's axis passes, once a turn handle set it, with a
/// button back to the origin.
pub(crate) fn turn_centre_row(ui: &mut egui::Ui, look: Look, centre: &mut Option<[f64; 3]>) {
    if centre.is_none() {
        return;
    }
    widgets::field_label(ui, look, "Through");
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("The handle's centre").color(look.c(|t| t.text)));
        if ui
            .small_button("Origin")
            .on_hover_text("Turn about an axis through the origin instead")
            .clicked()
        {
            *centre = None;
        }
    });
    ui.end_row();
}

impl ChamferaApp {
    /// A length field in mm (a plain number in the design's unit), if it reads.
    fn field_mm(&self, s: &str) -> Option<f64> {
        let s = if s.trim().is_empty() { "0" } else { s.trim() };
        let unit = self.session.doc.units.length.mm();
        expr::evaluate_as_in(s, Dim::LENGTH, unit, &expr::NoNames)
            .ok()
            .filter(|v| v.is_finite())
    }

    /// An angle field in radians (a plain number in degrees), if it reads.
    fn field_rad(&self, s: &str) -> Option<f64> {
        let s = if s.trim().is_empty() { "0" } else { s.trim() };
        expr::evaluate_as(s, Dim::ANGLE, &expr::NoNames)
            .ok()
            .filter(|v| v.is_finite())
    }

    /// The viewport as the manipulator math sees it.
    fn manip_view(&self, rect: Rect) -> Option<ScreenView> {
        ScreenView::new(
            self.session.camera,
            f64::from(rect.width()),
            f64::from(rect.height()),
            self.session.scene.bounds(),
        )
    }

    /// The centre of the bounding box of the solids `which` accepts.
    fn solids_centre(
        &self,
        which: impl Fn(&chamfera_engine::solids::Solid) -> bool,
    ) -> Option<Point3> {
        self.session
            .solids()
            .iter()
            .filter(|s| which(s))
            .filter_map(|s| s.body.bounding_box())
            .reduce(Aabb::union)
            .map(|b| b.center())
    }

    /// The handle of the open tool, if it has one: Extrude (a distance), Move, Move
    /// Component, then Fillet or Chamfer.
    pub(crate) fn manip_handle(&self) -> Option<(Target, Handle)> {
        if let Some(tool) = &self.solid.extrude
            && tool.form.extent == chamfera_doc::ExtrudeExtent::Distance
            && let Some((sketch, seed)) = tool.first_profile()
            && let Ok(f) = self.session.doc.sketch(sketch)
        {
            let n = f.plane.z();
            // The arrow stands in the middle of the profile: the centre of its outline
            // (each segment's middle weighted by its length), when that lies inside the
            // same profile; else where it was clicked.
            let (mut sum, mut total) = (Vec3::ZERO, 0.0);
            for [a, b] in self.session.profile_outline(sketch, seed) {
                let l = (b - a).length();
                sum = sum + ((a - Point3::ORIGIN) + (b - Point3::ORIGIN)) * (0.5 * l);
                total += l;
            }
            let region = f.sketch.region_at(seed);
            let seed = Some(total)
                .filter(|t| *t > 0.0)
                .map(|t| f.plane.to_plane(Point3::ORIGIN + sum * (1.0 / t)))
                .filter(|c| region.is_some() && f.sketch.region_at(*c) == region)
                .unwrap_or(seed);
            let offset = self.field_mm(&tool.form.start_offset).unwrap_or(0.0);
            let (dir, per_value) = match tool.form.direction {
                ExtrudeDirection::Normal | ExtrudeDirection::TwoSides => (n, 1.0),
                ExtrudeDirection::Reversed => (-n, 1.0),
                ExtrudeDirection::Symmetric => (n, 0.5),
            };
            return Some((
                Target::Extrude,
                Handle::Value {
                    origin: f.plane.from_plane(seed) + n * offset,
                    dir,
                    per_value,
                    value: self.field_mm(&tool.form.distance).unwrap_or(0.0),
                    stem: false,
                },
            ));
        }
        if let Some(tool) = &self.solid.transform
            && tool.kind == crate::transform_ui::TransformKind::Move
            && !tool.bodies.is_empty()
        {
            let pivot = self.solids_centre(|s| tool.bodies.contains(&s.name))?;
            let centre = if tool.rotate {
                pivot
            } else {
                let d = [&tool.dx, &tool.dy, &tool.dz].map(|v| self.field_mm(v).unwrap_or(0.0));
                pivot + Vec3::new(d[0], d[1], d[2])
            };
            return Some((Target::Move, Handle::Triad { centre, pivot }));
        }
        if let Some(crate::assembly_edit_ui::AssemblyDialog::MoveComponent {
            component,
            dx,
            dy,
            dz,
            absolute: false,
            ..
        }) = &self.assembly.edit
        {
            let assembly = &self.session.doc.assembly;
            let pivot = self.solids_centre(|s| assembly.is_inside(s.component, *component))?;
            let d = [dx, dy, dz].map(|v| self.field_mm(v).unwrap_or(0.0));
            return Some((
                Target::Component(*component),
                Handle::Triad {
                    centre: pivot + Vec3::new(d[0], d[1], d[2]),
                    pivot,
                },
            ));
        }
        if let Some(tool) = &self.solid.blend
            && let Some(edge) = tool.edges.first()
            && let Some((origin, dir)) = self.session.edge_handle(edge)
        {
            return Some((
                Target::Blend,
                Handle::Value {
                    origin,
                    dir,
                    per_value: 1.0,
                    value: self.field_mm(&tool.size).unwrap_or(0.0),
                    stem: true,
                },
            ));
        }
        None
    }

    /// The arrow of a one-value handle.
    fn value_arrow(view: &ScreenView, handle: &Handle) -> Option<Arrow> {
        let Handle::Value { dir, .. } = *handle else {
            return None;
        };
        let base = handle.grip();
        Some(Arrow {
            base,
            dir,
            length: view.screen_size(base, HANDLE_PIXELS * 0.8)?,
        })
    }

    /// The triad drawn at `centre`.
    fn triad(view: &ScreenView, centre: Point3) -> Option<Triad> {
        Some(Triad {
            centre,
            size: view.screen_size(centre, HANDLE_PIXELS)?,
        })
    }

    /// The part of `handle` under viewport pixel `p`.
    fn manip_hit(view: &ScreenView, handle: &Handle, p: Pixel) -> Option<Part> {
        match handle {
            Handle::Value { .. } => {
                let arrow = ChamferaApp::value_arrow(view, handle)?;
                let on_grip = view
                    .project(handle.grip())
                    .is_some_and(|g| (g[0] - p[0]).hypot(g[1] - p[1]) <= GRAB_PIXELS);
                (on_grip || arrow.hit(view, p, GRAB_PIXELS)).then_some(Part::Value)
            }
            Handle::Triad { centre, .. } => ChamferaApp::triad(view, *centre)?
                .hit(view, p, GRAB_PIXELS)
                .map(Part::Triad),
        }
    }

    /// True when the left button went down on a handle: the press belongs to it, not to
    /// orbiting or a selection box.
    pub(crate) fn manip_grabs(&self, ui: &egui::Ui, rect: Rect) -> bool {
        let Some(origin) = ui.input(|i| i.pointer.press_origin()) else {
            return false;
        };
        let (Some(view), Some((_, handle))) = (self.manip_view(rect), self.manip_handle()) else {
            return false;
        };
        let p = [
            f64::from(origin.x - rect.min.x),
            f64::from(origin.y - rect.min.y),
        ];
        ChamferaApp::manip_hit(&view, &handle, p).is_some()
    }

    /// The values a drag of `part` starts from: the distance or size (mm); the three
    /// distances (mm); or the turn so far about that axis (radians, in `[0]`).
    fn manip_start(&self, target: Target, part: Part) -> [f64; 3] {
        let len = |s: &str| self.field_mm(s).unwrap_or(0.0);
        match (target, part) {
            (Target::Extrude, _) => [
                self.solid
                    .extrude
                    .as_ref()
                    .map_or(0.0, |t| len(&t.form.distance)),
                0.0,
                0.0,
            ],
            (Target::Blend, _) => [
                self.solid.blend.as_ref().map_or(0.0, |t| len(&t.size)),
                0.0,
                0.0,
            ],
            (Target::Move, Part::Triad(TriadPart::Ring(i))) => {
                let a = self
                    .solid
                    .transform
                    .as_ref()
                    .filter(|t| t.rotate && t.axis == i)
                    .and_then(|t| self.field_rad(&t.angle))
                    .unwrap_or(0.0);
                [a, 0.0, 0.0]
            }
            (Target::Move, _) => match &self.solid.transform {
                Some(t) if !t.rotate => [len(&t.dx), len(&t.dy), len(&t.dz)],
                _ => [0.0; 3],
            },
            (Target::Component(_), part) => match &self.assembly.edit {
                Some(crate::assembly_edit_ui::AssemblyDialog::MoveComponent {
                    dx,
                    dy,
                    dz,
                    axis,
                    angle,
                    ..
                }) => match part {
                    Part::Triad(TriadPart::Ring(i)) => [
                        if *axis == i {
                            self.field_rad(angle).unwrap_or(0.0)
                        } else {
                            0.0
                        },
                        0.0,
                        0.0,
                    ],
                    _ => [len(dx), len(dy), len(dz)],
                },
                _ => [0.0; 3],
            },
        }
    }

    /// Writes a drag into the dialog's fields: `start` plus `delta` (mm along the value
    /// or the axes, or radians of turn in `[0]`), snapped to `step` mm or `angle_step`.
    fn manip_set(
        &mut self,
        target: Target,
        part: Part,
        pivot: Point3,
        start: [f64; 3],
        delta: [f64; 3],
        (step, angle_step): (f64, f64),
    ) {
        let mut v = start;
        let mut turn = None;
        match part {
            Part::Value => v[0] = m::snap(start[0] + delta[0], step),
            Part::Triad(TriadPart::Axis(i)) => {
                if let (Some(s), Some(d)) = (start.get(i), delta.get(i))
                    && let Some(slot) = v.get_mut(i)
                {
                    *slot = m::snap(s + d, step);
                }
            }
            Part::Triad(TriadPart::Plane(i)) => {
                for j in others(i) {
                    if let (Some(s), Some(d)) = (start.get(j), delta.get(j))
                        && let Some(slot) = v.get_mut(j)
                    {
                        *slot = m::snap(s + d, step);
                    }
                }
            }
            Part::Triad(TriadPart::Ring(i)) => {
                turn = Some((i, m::snap(start[0] + delta[0], angle_step)));
            }
        }
        let centre = [pivot.x, pivot.y, pivot.z];
        match target {
            Target::Extrude => {
                if let Some(t) = self.solid.extrude.as_mut() {
                    t.form.distance = length_text(v[0]);
                }
            }
            Target::Blend => {
                if let Some(t) = self.solid.blend.as_mut() {
                    // A size is positive: the handle stops one step out of the edge.
                    t.size = length_text(v[0].max(step));
                }
            }
            Target::Move => {
                let Some(t) = self.solid.transform.as_mut() else {
                    return;
                };
                match turn {
                    Some((i, a)) => {
                        t.rotate = true;
                        t.axis = i;
                        t.angle = angle_text(a);
                        t.centre = Some(centre);
                    }
                    None => {
                        t.rotate = false;
                        [t.dx, t.dy, t.dz] = v.map(length_text);
                    }
                }
            }
            Target::Component(_) => {
                if let Some(crate::assembly_edit_ui::AssemblyDialog::MoveComponent {
                    dx,
                    dy,
                    dz,
                    axis,
                    angle,
                    centre: c,
                    ..
                }) = self.assembly.edit.as_mut()
                {
                    match turn {
                        Some((i, a)) => {
                            *axis = i;
                            // The dialog reads a plain angle as degrees.
                            *angle = trimmed(a.to_degrees());
                            *c = Some(centre);
                        }
                        None => [*dx, *dy, *dz] = v.map(length_text),
                    }
                }
            }
        }
    }

    /// The length and angle steps a drag snaps to now: steps of the drawn grid at the
    /// handle (a tenth with Shift or with grid snapping off), and 5 (Shift: 1) degrees.
    fn manip_steps(&self, view: &ScreenView, at: Point3, fine: bool) -> (f64, f64) {
        let grid = chamfera_gpu::grid_spacing(
            self.session.camera.distance,
            self.session.scene.grid_spacing,
        );
        let k = view.mm_per_pixel(at).unwrap_or(0.0);
        let step = m::drag_step(grid, k, fine || !self.prefs.snap_to_grid);
        (step, if fine { FINE_ANGLE_STEP } else { ANGLE_STEP })
    }

    /// Viewport input for the handles: a left drag that starts on one changes the
    /// dialog. True when the input was used.
    pub(crate) fn manip_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: Rect,
    ) -> bool {
        let local = |p: Pos2| [f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y)];
        let (Some(view), Some((target, handle))) = (self.manip_view(rect), self.manip_handle())
        else {
            let s = &mut self.solid.manip;
            s.drag = None;
            s.hover = None;
            s.shown = None;
            return false;
        };
        if let Some(spec) = self.solid.manip.demo.take() {
            match parse_demo(&spec) {
                Ok((part, delta)) => {
                    let start = self.manip_start(target, part);
                    let pivot = match handle {
                        Handle::Triad { pivot, .. } => pivot,
                        Handle::Value { .. } => handle.grip(),
                    };
                    self.manip_set(target, part, pivot, start, delta, (0.0, 0.0));
                    self.solid.manip.shown = Some(part);
                }
                Err(e) => self.message = Some((e, true)),
            }
            return true;
        }
        if self.solid.manip.drag.is_none() {
            self.solid.manip.hover = response
                .hover_pos()
                .and_then(|p| ChamferaApp::manip_hit(&view, &handle, local(p)));
        }
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(p) = ui.input(|i| i.pointer.press_origin())
            && let Some(part) = ChamferaApp::manip_hit(&view, &handle, local(p))
        {
            let start = self.manip_start(target, part);
            self.solid.manip.drag = Some(Drag {
                target,
                part,
                from: local(p),
                last: local(p),
                turned: 0.0,
                start,
                view,
                handle,
            });
            self.solid.manip.shown = None;
            return true;
        }
        let Some(mut drag) = self.solid.manip.drag else {
            return false;
        };
        if drag.target != target {
            self.solid.manip.drag = None;
            return false;
        }
        if response.dragged()
            && let Some(p) = response.interact_pointer_pos()
        {
            let to = local(p);
            let fine = ui.input(|i| i.modifiers.shift);
            if let Some(delta) = ChamferaApp::manip_delta(&mut drag, to) {
                let pivot = match drag.handle {
                    Handle::Triad { pivot, .. } => pivot,
                    Handle::Value { .. } => drag.handle.grip(),
                };
                let steps = self.manip_steps(&drag.view, drag.handle.grip(), fine);
                self.manip_set(target, drag.part, pivot, drag.start, delta, steps);
            }
            self.solid.manip.drag = Some(drag);
        }
        if response.drag_stopped() {
            self.solid.manip.drag = None;
        }
        true
    }

    /// What the pointer at `to` means for `drag`, in model units from where it started:
    /// mm along the value or the axes, or the turn (radians, in `[0]`) accumulated
    /// through every step so it can pass half a turn.
    fn manip_delta(drag: &mut Drag, to: Pixel) -> Option<[f64; 3]> {
        let view = &drag.view;
        match (drag.part, drag.handle) {
            (
                Part::Value,
                Handle::Value {
                    origin,
                    dir,
                    per_value,
                    ..
                },
            ) => {
                let d = m::axis_drag(view, origin, dir, drag.from, to)?;
                (per_value > 0.0).then(|| [d / per_value, 0.0, 0.0])
            }
            (Part::Triad(TriadPart::Axis(i)), Handle::Triad { centre, .. }) => {
                let d = m::axis_drag(view, centre, Triad::axis(i), drag.from, to)?;
                let mut out = [0.0; 3];
                if let Some(s) = out.get_mut(i) {
                    *s = d;
                }
                Some(out)
            }
            (Part::Triad(TriadPart::Plane(i)), Handle::Triad { centre, .. }) => {
                let v = m::plane_drag(view, centre, Triad::axis(i), drag.from, to)?;
                Some([v.x, v.y, v.z])
            }
            (Part::Triad(TriadPart::Ring(i)), Handle::Triad { centre, .. }) => {
                if let Some(a) = m::ring_angle(view, centre, Triad::axis(i), drag.last, to) {
                    drag.turned += a;
                    drag.last = to;
                }
                Some([drag.turned, 0.0, 0.0])
            }
            _ => None,
        }
    }

    /// Keeps the translucent preview of a Move or Move Component in step with its
    /// dialog (`solid.move` or `component.move` with `preview`); removes it when the
    /// tool closes. True when it changed.
    pub(crate) fn update_manip_preview(&mut self) -> bool {
        let zero = |s: &str, f: &dyn Fn(&str) -> Option<f64>| f(s).is_some_and(|v| v == 0.0);
        let len = |s: &str| self.field_mm(s);
        let ang = |s: &str| self.field_rad(s);
        let mut wanted: Option<(&str, Value)> = None;
        if let Some(t) = &self.solid.transform
            && t.kind == crate::transform_ui::TransformKind::Move
            && let Some(p) = t.params()
        {
            let still = if t.rotate {
                zero(&t.angle, &ang)
            } else {
                [&t.dx, &t.dy, &t.dz].iter().all(|v| zero(v, &len))
            };
            if !still {
                wanted = Some(("solid.move", p));
            }
        }
        if let Some(crate::assembly_edit_ui::AssemblyDialog::MoveComponent {
            dx,
            dy,
            dz,
            angle,
            absolute: false,
            ..
        }) = &self.assembly.edit
        {
            let still = [dx, dy, dz].iter().all(|v| zero(v, &len)) && zero(angle, &ang);
            if !still && let Ok((id, p)) = self.assembly_dialog_params() {
                wanted = Some((id, p));
            }
        }
        let key = wanted.as_ref().map(|(id, p)| json!([id, p]));
        if key == self.solid.manip.preview_for {
            return false;
        }
        match wanted {
            Some((id, mut p)) => {
                p["preview"] = json!(true);
                if self.session.execute(id, &p).is_err() {
                    let _ = self
                        .session
                        .execute("solid.move", &json!({ "clear": true }));
                }
            }
            None => {
                let _ = self
                    .session
                    .execute("solid.move", &json!({ "clear": true }));
            }
        }
        self.solid.manip.preview_for = key;
        self.preview_version += 1;
        true
    }

    /// Draws the open tool's handle over the viewport, and keeps the move preview in
    /// step.
    pub(crate) fn manipulators(&mut self, ui: &egui::Ui, rect: Rect) {
        if self.update_manip_preview() {
            ui.ctx().request_repaint();
        }
        let (Some(view), Some((target, handle))) = (self.manip_view(rect), self.manip_handle())
        else {
            return;
        };
        let state = &self.solid.manip;
        let active = state
            .drag
            .filter(|d| d.target == target)
            .map(|d| d.part)
            .or(state.hover)
            .or(state.shown);
        let at = |p: Point3| {
            view.project(p)
                .map(|q| rect.min + Vec2::new(q[0] as f32, q[1] as f32))
        };
        let painter = ui.painter_at(rect);
        let t = &self.theme;
        let lit = color(t.hover_highlight);
        match handle {
            Handle::Value { origin, stem, .. } => {
                let Some(arrow) = ChamferaApp::value_arrow(&view, &handle) else {
                    return;
                };
                let c = if active.is_some() {
                    lit
                } else {
                    color(t.highlight)
                };
                if stem && let (Some(a), Some(b)) = (at(origin), at(arrow.base)) {
                    painter.line_segment([a, b], Stroke::new(1.5, c));
                }
                if let (Some(a), Some(b)) = (at(arrow.base), at(arrow.tip())) {
                    paint_arrow(&painter, a, b, c, 2.5);
                    painter.circle(a, 4.5, color(t.panel), Stroke::new(2.0, c));
                    let text = self.manip_label(target, Part::Value);
                    self.manip_text(&painter, b, &text, c, false);
                }
            }
            Handle::Triad { centre, .. } => {
                let Some(triad) = ChamferaApp::triad(&view, centre) else {
                    return;
                };
                let axis = [color(t.axis_x), color(t.axis_y), color(t.axis_z)];
                let pick = |part: TriadPart, i: usize| {
                    if active == Some(Part::Triad(part)) {
                        lit
                    } else {
                        axis.get(i).copied().unwrap_or(lit)
                    }
                };
                for i in 0..3 {
                    let arc: Option<Vec<Pos2>> = triad.ring(i).into_iter().map(at).collect();
                    if let Some(arc) = arc {
                        let c = pick(TriadPart::Ring(i), i);
                        painter.add(Shape::line(arc, Stroke::new(2.5, c)));
                    }
                }
                for i in 0..3 {
                    let square: Option<Vec<Pos2>> =
                        triad.plane_square(i).into_iter().map(at).collect();
                    if let Some(square) = square {
                        let c = pick(TriadPart::Plane(i), i);
                        painter.add(Shape::convex_polygon(
                            square,
                            c.gamma_multiply(0.4),
                            Stroke::new(1.0, c),
                        ));
                    }
                }
                for i in 0..3 {
                    let a = triad.arrow(i);
                    if let (Some(p), Some(q)) = (at(a.base), at(a.tip())) {
                        paint_arrow(&painter, p, q, pick(TriadPart::Axis(i), i), 2.5);
                    }
                }
                if let Some(c) = at(centre) {
                    painter.circle(c, 4.0, color(t.panel), Stroke::new(1.5, color(t.text)));
                    if let Some(Part::Triad(part)) = active {
                        let text = self.manip_label(target, Part::Triad(part));
                        self.manip_text(&painter, c, &text, lit, true);
                    }
                }
            }
        }
    }

    /// What a handle part sets, with its value as the dialog shows it.
    fn manip_label(&self, target: Target, part: Part) -> String {
        const AXES: [&str; 3] = ["X", "Y", "Z"];
        let name = |i: usize| AXES.get(i).copied().unwrap_or("");
        match target {
            Target::Extrude => self.solid.extrude.as_ref().map_or_else(String::new, |t| {
                format!("Distance {}", t.form.distance.trim())
            }),
            Target::Blend => self.solid.blend.as_ref().map_or_else(String::new, |t| {
                let what = if t.chamfer { "Distance" } else { "Radius" };
                format!("{what} {}", t.size.trim())
            }),
            Target::Move | Target::Component(_) => {
                let (d, angle) = match (&self.solid.transform, &self.assembly.edit) {
                    (Some(t), _) if target == Target::Move => {
                        ([t.dx.clone(), t.dy.clone(), t.dz.clone()], t.angle.clone())
                    }
                    (
                        _,
                        Some(crate::assembly_edit_ui::AssemblyDialog::MoveComponent {
                            dx,
                            dy,
                            dz,
                            angle,
                            ..
                        }),
                    ) => (
                        [dx.clone(), dy.clone(), dz.clone()],
                        format!("{} deg", angle.trim()),
                    ),
                    _ => return String::new(),
                };
                let get = |i: usize| d.get(i).map_or("", |s| s.trim()).to_owned();
                match part {
                    Part::Triad(TriadPart::Axis(i)) => format!("{} {}", name(i), get(i)),
                    Part::Triad(TriadPart::Plane(i)) => {
                        let [a, b] = others(i);
                        let (a, b) = (a.min(b), a.max(b));
                        format!("{} {} · {} {}", name(a), get(a), name(b), get(b))
                    }
                    Part::Triad(TriadPart::Ring(i)) => {
                        format!("Turn about {} {}", name(i), angle.trim())
                    }
                    Part::Value => String::new(),
                }
            }
        }
    }

    /// A value label in a small panel to the right of `at` and a little above, or (with
    /// `left`) to its left, clear of a triad whose centre is `at`.
    fn manip_text(&self, painter: &egui::Painter, at: Pos2, text: &str, edge: Color32, left: bool) {
        if text.is_empty() {
            return;
        }
        let galley = painter.layout_no_wrap(
            text.to_owned(),
            egui::FontId::proportional(12.5),
            color(self.theme.text),
        );
        let half = galley.size().x * 0.5;
        let centre = if left {
            at - Vec2::new(half + 28.0, 0.0)
        } else {
            at + Vec2::new(half + 16.0, -12.0)
        };
        let r = Rect::from_center_size(centre, galley.size() + Vec2::new(12.0, 6.0));
        let radius = egui::CornerRadius::same(widgets::R_SMALL);
        painter.rect_filled(r, radius, color(self.theme.panel).gamma_multiply(0.96));
        painter.rect_stroke(r, radius, Stroke::new(1.5, edge), egui::StrokeKind::Inside);
        painter.galley(
            r.center() - galley.size() * 0.5,
            galley,
            color(self.theme.text),
        );
    }
}

/// An arrow on screen from `a` to `b`: a shaft and a filled head.
fn paint_arrow(painter: &egui::Painter, a: Pos2, b: Pos2, c: Color32, width: f32) {
    let d = b - a;
    let len = d.length();
    if len < 1.0 {
        return;
    }
    let u = d / len;
    let head = 13.0f32.min(len * 0.5);
    let side = Vec2::new(-u.y, u.x) * (head * 0.42);
    let back = b - u * head;
    painter.line_segment([a, back], Stroke::new(width, c));
    painter.add(Shape::convex_polygon(
        vec![b, back + side, back - side],
        c,
        Stroke::NONE,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform_ui::TransformKind;

    const W: f32 = 800.0;
    const H: f32 = 600.0;

    fn rect() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(W, H))
    }

    /// A headless app with a box x 0..50, y 0..30, z 0..10 (sketch 1, extrude 2), seen
    /// from the front, orthographic: X runs right and Z up on the screen.
    fn app_with_box() -> ChamferaApp {
        let mut app = ChamferaApp::headless();
        app.run("sketch.create", json!({ "plane": "xy" }));
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 50, "y2": 30 }),
        );
        app.run("sketch.finish", json!({}));
        app.run(
            "solid.extrude",
            json!({ "sketch": 1, "profiles": [{ "x": 25, "y": 15 }], "distance": 10 }),
        );
        app.run("view.projection", json!({ "mode": "orthographic" }));
        app.run("view.standard", json!({ "view": "front" }));
        app
    }

    fn px(app: &ChamferaApp, p: Point3) -> Pixel {
        app.manip_view(rect()).unwrap().project(p).unwrap()
    }

    /// Drags `part` of the open handle from pixel `from` to `to`, as the input does.
    fn drag(app: &mut ChamferaApp, from: Pixel, to: Pixel, fine: bool) -> Part {
        let view = app.manip_view(rect()).unwrap();
        let (target, handle) = app.manip_handle().unwrap();
        let part = ChamferaApp::manip_hit(&view, &handle, from).expect("nothing grabbed");
        let mut d = Drag {
            target,
            part,
            from,
            last: from,
            turned: 0.0,
            start: app.manip_start(target, part),
            view,
            handle,
        };
        let delta = ChamferaApp::manip_delta(&mut d, to).unwrap();
        let pivot = match handle {
            Handle::Triad { pivot, .. } => pivot,
            Handle::Value { .. } => handle.grip(),
        };
        let steps = app.manip_steps(&view, handle.grip(), fine);
        app.manip_set(target, part, pivot, d.start, delta, steps);
        part
    }

    #[test]
    fn the_demo_spec_reads_every_part() {
        assert_eq!(
            parse_demo("value=15").unwrap(),
            (Part::Value, [15.0, 0.0, 0.0])
        );
        assert_eq!(
            parse_demo("y=-4").unwrap(),
            (Part::Triad(TriadPart::Axis(1)), [0.0, -4.0, 0.0])
        );
        assert_eq!(
            parse_demo("xz=3,7").unwrap(),
            (Part::Triad(TriadPart::Plane(1)), [3.0, 0.0, 7.0])
        );
        let (p, d) = parse_demo("turn-z=90").unwrap();
        assert_eq!(p, Part::Triad(TriadPart::Ring(2)));
        assert!((d[0] - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        for bad in [
            "", "x", "x=", "x=1,2", "xy=1", "w=3", "x=nan", "x=1e300", "turn-q=1",
        ] {
            assert!(parse_demo(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn dragging_the_extrude_arrow_sets_the_distance_and_the_preview_follows() {
        let mut app = ChamferaApp::headless();
        app.run("sketch.create", json!({ "plane": "xy" }));
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 50, "y2": 30 }),
        );
        app.run("sketch.finish", json!({}));
        app.run("view.projection", json!({ "mode": "orthographic" }));
        // Pick the profile from the iso view (the front view sees its plane edge-on),
        // then drag from the front, where Z runs straight up the screen.
        app.run("view.standard", json!({ "view": "iso" }));
        app.open_extrude(false);
        let p = px(&app, Point3::new(25.0, 15.0, 0.0));
        app.extrude_click(p[0], p[1], f64::from(W), f64::from(H));
        app.run("view.standard", json!({ "view": "front" }));
        let (target, handle) = app.manip_handle().unwrap();
        assert_eq!(target, Target::Extrude);
        // The arrow stands on the 10 mm cap (the default distance) and points up +Z.
        assert!((handle.grip() - Point3::new(25.0, 15.0, 10.0)).length() < 1e-9);
        let view = app.manip_view(rect()).unwrap();
        let k = view.mm_per_pixel(Point3::ORIGIN).unwrap();
        // From the middle of the arrow, 26.4 mm worth of pixels up the screen: the
        // distance becomes 10 + 26.4, snapped to the step (grid share, see drag_step).
        let grip = px(&app, handle.grip());
        let from = [grip[0], grip[1] - 40.0];
        let to = [from[0], from[1] - 26.4 / k];
        drag(&mut app, from, to, false);
        let (step, _) = app.manip_steps(&view, handle.grip(), false);
        let want = ((36.4 / step).round() * step * 1e4).round() / 1e4;
        let got = app
            .field_mm(&app.solid.extrude.as_ref().unwrap().form.distance)
            .unwrap();
        assert!((got - want).abs() < 1e-9, "{got} vs {want} (step {step})");
        // Shift: a tenth of the step, so 36.4 within 0.05.
        app.solid.extrude.as_mut().unwrap().form.distance = "10 mm".into();
        drag(&mut app, from, to, true);
        let fine = app
            .field_mm(&app.solid.extrude.as_ref().unwrap().form.distance)
            .unwrap();
        assert!((fine - 36.4).abs() <= step / 20.0 + 1e-9, "{fine}");
        // The preview is built from the dialog's parameters: 50 * 30 * the distance,
        // and nothing is added to the design (no undo step) until OK.
        let mark = app.session.undo_mark();
        assert!(app.update_extrude_preview());
        let v = app.session.scene.previews[0].signed_volume();
        assert!((v - 1500.0 * fine).abs() < 1e-6, "{v}");
        assert_eq!(app.session.group_undo_since(mark), 0);
    }

    #[test]
    fn the_move_triad_drags_along_an_axis_and_in_a_plane_and_previews() {
        let mut app = app_with_box();
        app.open_transform(TransformKind::Move);
        app.solid.transform.as_mut().unwrap().bodies = vec!["Extrude1".into()];
        let (target, handle) = app.manip_handle().unwrap();
        assert_eq!(target, Target::Move);
        // The triad sits at the box's centre (25, 15, 5).
        let Handle::Triad { centre, .. } = handle else {
            panic!("{handle:?}")
        };
        assert!((centre - Point3::new(25.0, 15.0, 5.0)).length() < 1e-9);
        let view = app.manip_view(rect()).unwrap();
        let k = view.mm_per_pixel(centre).unwrap();
        let c = px(&app, centre);
        // Grab X 60 pixels right of the centre and drag 20 mm worth further right.
        let part = drag(
            &mut app,
            [c[0] + 60.0, c[1]],
            [c[0] + 60.0 + 20.0 / k, c[1]],
            false,
        );
        assert_eq!(part, Part::Triad(TriadPart::Axis(0)));
        let t = app.solid.transform.as_ref().unwrap();
        assert_eq!(
            (t.dx.as_str(), t.dy.as_str(), t.dz.as_str()),
            ("20 mm", "0 mm", "0 mm")
        );
        // The preview shows the box moved 20 along X: x 20..70, and adds no undo step.
        let mark = app.session.undo_mark();
        assert!(app.update_manip_preview());
        let b = chamfera_geom::Aabb::from_points(
            app.session.scene.previews[0].positions.iter().copied(),
        )
        .unwrap();
        assert!(
            (b.min.x - 20.0).abs() < 1e-6 && (b.max.x - 70.0).abs() < 1e-6,
            "{b:?}"
        );
        assert_eq!(app.session.group_undo_since(mark), 0);
        // The XZ square (normal Y) is seen face-on from the front: drag it 10 right and
        // 6 up from its middle (0.4 of the size out along X and Z from the new centre);
        // 6, not 5, since the step here is 2 mm (a 10 mm grid at 0.26 mm a pixel).
        let (_, handle) = app.manip_handle().unwrap();
        let Handle::Triad { centre, .. } = handle else {
            panic!()
        };
        let size = view.screen_size(centre, HANDLE_PIXELS).unwrap();
        let mid = px(&app, centre + Vec3::new(0.4 * size, 0.0, 0.4 * size));
        let part = drag(&mut app, mid, [mid[0] + 10.0 / k, mid[1] - 6.0 / k], false);
        assert_eq!(part, Part::Triad(TriadPart::Plane(1)));
        let t = app.solid.transform.as_ref().unwrap();
        assert_eq!((t.dx.as_str(), t.dz.as_str()), ("30 mm", "6 mm"));
        // OK: one Move feature, one undo step, the box at x 30..80, z 6..16.
        let before = app.session.undo_mark();
        app.transform_ok().unwrap();
        assert_eq!(app.session.group_undo_since(before), 1);
        let b = app.session.solids()[0].body.bounding_box().unwrap();
        assert!(
            (b.min.x - 30.0).abs() < 1e-9 && (b.min.z - 6.0).abs() < 1e-9,
            "{b:?}"
        );
        // The tool closed: the preview goes.
        assert!(app.update_manip_preview());
        assert!(app.session.scene.previews.is_empty());
    }

    #[test]
    fn the_move_turn_arc_turns_about_the_triad_centre() {
        let mut app = app_with_box();
        app.open_transform(TransformKind::Move);
        app.solid.transform.as_mut().unwrap().bodies = vec!["Extrude1".into()];
        let (_, handle) = app.manip_handle().unwrap();
        let Handle::Triad { centre, .. } = handle else {
            panic!()
        };
        // The arc about Y (between Z and X) faces the front view. Grab it where it
        // crosses 45 degrees and sweep to 45 + 90 degrees: about Y from Z toward X is
        // positive, and on screen that is clockwise; the tool rounds to 5 degrees.
        let view = app.manip_view(rect()).unwrap();
        let size = view.screen_size(centre, HANDLE_PIXELS).unwrap();
        let r = 0.8 * size;
        let at = |deg: f64| {
            let a = deg.to_radians();
            // Ring about Y runs from Z (a = 0) to X (a = 90).
            px(&app, centre + Vec3::new(r * a.sin(), 0.0, r * a.cos()))
        };
        let (from, to) = (at(45.0), at(135.0));
        let part = drag(&mut app, from, to, false);
        assert_eq!(part, Part::Triad(TriadPart::Ring(1)));
        let t = app.solid.transform.as_ref().unwrap();
        assert!(t.rotate && t.axis == 1, "{:?}", (t.rotate, t.axis));
        assert_eq!(t.angle, "90 deg");
        assert_eq!(t.centre, Some([25.0, 15.0, 5.0]));
        // A quarter turn about the Y axis through (25, 15, 5): the 50 x 10 (x by z)
        // section becomes 10 x 50, centred there: x 20..30, z -20..30.
        app.transform_ok().unwrap();
        let b = app.session.solids()[0].body.bounding_box().unwrap();
        assert!(
            (b.min.x - 20.0).abs() < 1e-6 && (b.max.x - 30.0).abs() < 1e-6,
            "{b:?}"
        );
        assert!(
            (b.min.z + 20.0).abs() < 1e-6 && (b.max.z - 30.0).abs() < 1e-6,
            "{b:?}"
        );
    }

    #[test]
    fn the_fillet_handle_sits_on_the_edge_and_drags_the_radius() {
        let mut app = app_with_box();
        app.open_blend(false);
        // The top front edge: y = 0, z = 10, x 0..50; its middle is (25, 0, 10) and the
        // inward bisector is (0, 1, -1) / sqrt 2.
        // Picked in the iso view, where it is not behind the top back edge.
        app.run("view.standard", json!({ "view": "iso" }));
        let name = {
            let p = px(&app, Point3::new(25.0, 0.0, 10.0));
            app.session
                .edge_at(p[0], p[1], f64::from(W), f64::from(H))
                .unwrap()
                .2
        };
        app.run("view.standard", json!({ "view": "front" }));
        app.solid.blend.as_mut().unwrap().edges = vec![name];
        let (target, handle) = app.manip_handle().unwrap();
        assert_eq!(target, Target::Blend);
        let Handle::Value { origin, dir, .. } = handle else {
            panic!()
        };
        assert!(
            (origin - Point3::new(25.0, 0.0, 10.0)).length() < 1e-9,
            "{origin:?}"
        );
        let s = 0.5f64.sqrt();
        assert!((dir - Vec3::new(0.0, s, -s)).length() < 1e-9, "{dir:?}");
        // From the front the bisector shows only its -Z part (1/sqrt 2 per mm): dragging
        // the grip 3 / sqrt 2 mm down the screen adds 3 mm to the 2 mm radius.
        let view = app.manip_view(rect()).unwrap();
        let k = view.mm_per_pixel(origin).unwrap();
        let g = px(&app, handle.grip());
        drag(&mut app, g, [g[0], g[1] + 3.0 * s / k], true);
        let r = app
            .field_mm(&app.solid.blend.as_ref().unwrap().size)
            .unwrap();
        assert!((r - 5.0).abs() < 0.06, "{r}");
        // Dragging far out never makes the radius zero or negative.
        let g = px(&app, app.manip_handle().unwrap().1.grip());
        drag(&mut app, g, [g[0], g[1] - 400.0], false);
        let r = app
            .field_mm(&app.solid.blend.as_ref().unwrap().size)
            .unwrap();
        assert!(r > 0.0, "{r}");
    }

    #[test]
    fn the_component_triad_writes_the_move_component_fields() {
        let mut app = app_with_box();
        let c = app
            .run("component.new", json!({ "from_bodies": [2] }))
            .unwrap();
        let c = u32::try_from(c["component"].as_u64().unwrap()).unwrap();
        app.open_component_move(c);
        let (target, handle) = app.manip_handle().unwrap();
        assert_eq!(target, Target::Component(c));
        let Handle::Triad { centre, .. } = handle else {
            panic!()
        };
        let view = app.manip_view(rect()).unwrap();
        let k = view.mm_per_pixel(centre).unwrap();
        let p = px(&app, centre);
        // Z runs up the front view: 60 pixels up the arrow, drag 12 mm worth higher.
        drag(
            &mut app,
            [p[0], p[1] - 60.0],
            [p[0], p[1] - 60.0 - 12.0 / k],
            false,
        );
        let Some(crate::assembly_edit_ui::AssemblyDialog::MoveComponent { dz, .. }) =
            &app.assembly.edit
        else {
            panic!()
        };
        assert_eq!(dz, "12 mm");
        assert!(app.update_manip_preview());
        assert_eq!(app.session.scene.previews.len(), 1);
        let before = app.session.undo_mark();
        app.assembly_dialog_ok().unwrap();
        assert_eq!(app.session.group_undo_since(before), 1);
        let b = app.session.solids()[0].body.bounding_box().unwrap();
        assert!(
            (b.min.z - 12.0).abs() < 1e-9 && (b.max.z - 22.0).abs() < 1e-9,
            "{b:?}"
        );
    }
}
