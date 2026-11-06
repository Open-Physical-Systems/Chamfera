//! The ribbon: one table of every tool (`TOOLS`) in its workspace tab and group, and the
//! code that draws it. The ribbon's buttons, each group's dropdown and the search palette
//! all read the table, so they use the same names, icons, shortcuts, tooltips and
//! enabled checks, and run the tool the same way (`ChamferaApp::run_action`).
//!
//! A group shows its primary tools as big buttons; its caption is a dropdown listing
//! every tool of the group. When the window is too narrow for every group, groups
//! collapse from the right to their first tool and the dropdown (`plan`).
//!
//! What a tool does stays in its own file (`hole_ui.rs`, `cam_ui.rs`, ...); this file only
//! says where it sits and dispatches to it. A new tool gets a row here (and, if it has no
//! `Action` of its own, an arm in `run_tool`, `tool_active` and `tool_unavailable`).

use crate::ChamferaApp;
use crate::icons::Icon;
use crate::shortcuts::{self, Action};
use crate::sketch_ui::{PLANES, Tool as S};
use crate::widgets::{self, Look, S1, S2, S3};
use chamfera_doc::ROOT;
use chamfera_doc::drawing::{DimensionKind, ViewDirection};
use egui::{Rect, Response};
use serde_json::json;

/// A tab of the ribbon. Design has SOLID, and SKETCH while a sketch is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Solid,
    SheetMetal,
    Sketch,
    Drawing,
    Manufacture,
}

impl Tab {
    /// The tab's title, as the tab row shows it.
    pub fn title(self) -> &'static str {
        match self {
            Tab::Solid => "SOLID",
            Tab::SheetMetal => "SHEET METAL",
            Tab::Sketch => "SKETCH",
            Tab::Drawing => "DRAWING",
            Tab::Manufacture => "MANUFACTURE",
        }
    }

    /// The palette's context column for the tab's tools.
    pub fn context(self) -> &'static str {
        match self {
            Tab::Solid => "Solid",
            Tab::SheetMetal => "Sheet Metal",
            Tab::Sketch => "Sketch",
            Tab::Drawing => "Drawing",
            Tab::Manufacture => "Manufacture",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Tab::Solid => "Solid modelling: sketches, features, construction and inspection",
            Tab::SheetMetal => "Sheet metal: base flanges, flanges, flat patterns and rules",
            Tab::Sketch => "Editing a sketch: draw, constrain and dimension",
            Tab::Drawing => "2D drawing sheets: views of the model, dimensions, export",
            Tab::Manufacture => "Setups, 2D milling toolpaths and G-code",
        }
    }
}

/// A workspace, picked with the switcher at the ribbon's left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workspace {
    Design,
    Drawing,
    Manufacture,
}

impl Workspace {
    pub const ALL: [Workspace; 3] = [
        Workspace::Design,
        Workspace::Drawing,
        Workspace::Manufacture,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Workspace::Design => "Design",
            Workspace::Drawing => "Drawing",
            Workspace::Manufacture => "Manufacture",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Workspace::Design => Icon::Body,
            Workspace::Drawing => Icon::Drawing,
            Workspace::Manufacture => Icon::CamSetup,
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Workspace::Design => "Sketches, solids, assemblies",
            Workspace::Drawing => "2D sheets with views and dimensions",
            Workspace::Manufacture => "Toolpaths and G-code for milling",
        }
    }
}

/// A group of the ribbon, in the order the tab shows them.
#[derive(Debug, Clone, Copy)]
pub struct Group {
    pub tab: Tab,
    pub caption: &'static str,
    /// The tools after the primary ones are a two-row grid of small icons (CONSTRAINTS).
    pub grid: bool,
}

const fn group(tab: Tab, caption: &'static str) -> Group {
    Group {
        tab,
        caption,
        grid: false,
    }
}

pub const GROUPS: &[Group] = &[
    group(Tab::Solid, "CREATE"),
    group(Tab::Solid, "MODIFY"),
    group(Tab::Solid, "SURFACE"),
    group(Tab::Solid, "ASSEMBLE"),
    group(Tab::Solid, "CONSTRUCT"),
    group(Tab::Solid, "INSPECT"),
    group(Tab::Solid, "INSERT"),
    group(Tab::SheetMetal, "CREATE"),
    group(Tab::SheetMetal, "FLAT PATTERN"),
    group(Tab::SheetMetal, "SETUP"),
    group(Tab::Sketch, "SELECT"),
    group(Tab::Sketch, "CREATE"),
    Group {
        tab: Tab::Sketch,
        caption: "CONSTRAINTS",
        grid: true,
    },
    group(Tab::Sketch, "MODIFY"),
    group(Tab::Sketch, "INSERT"),
    group(Tab::Sketch, "FINISH SKETCH"),
    group(Tab::Drawing, "SHEET"),
    group(Tab::Drawing, "VIEWS"),
    group(Tab::Drawing, "DIMENSIONS"),
    group(Tab::Drawing, "EXPORT"),
    group(Tab::Manufacture, "SETUP"),
    group(Tab::Manufacture, "2D"),
    group(Tab::Manufacture, "ACTIONS"),
];

/// One tool of the ribbon.
#[derive(Debug, Clone, Copy)]
pub struct RibbonTool {
    /// What it does when clicked; also its key in the palette and the shortcut table.
    pub action: Action,
    pub tab: Tab,
    pub group: &'static str,
    /// Shown as a big button; the others are in the group's dropdown only.
    pub primary: bool,
    pub icon: Icon,
    /// The name in tooltips, dropdowns and the palette (`shortcuts::action_label`).
    pub name: &'static str,
    /// The big button's label when it is shorter than the name; empty for the name.
    pub short: &'static str,
    /// What it does: the tooltip's second line. Empty for sketch tools, which say it in
    /// `sketch_ui::Tool::tooltip`.
    pub what: &'static str,
    /// A separator goes above it in the dropdown.
    pub sep: bool,
}

const fn tool(
    tab: Tab,
    group: &'static str,
    action: Action,
    icon: Icon,
    name: &'static str,
    what: &'static str,
) -> RibbonTool {
    RibbonTool {
        action,
        tab,
        group,
        primary: false,
        icon,
        name,
        short: "",
        what,
        sep: false,
    }
}

impl RibbonTool {
    const fn primary(mut self) -> RibbonTool {
        self.primary = true;
        self
    }

    const fn short(mut self, short: &'static str) -> RibbonTool {
        self.short = short;
        self
    }

    const fn sep(mut self) -> RibbonTool {
        self.sep = true;
        self
    }

    /// The big button's label.
    pub fn label(&self) -> &'static str {
        if self.short.is_empty() {
            self.name
        } else {
            self.short
        }
    }

    /// What the tool does, for the tooltip.
    pub fn what(&self) -> &'static str {
        match self.action {
            Action::SketchTool(t) if self.what.is_empty() => {
                t.tooltip().split_once('\n').map_or("", |(_, rest)| rest)
            }
            _ => self.what,
        }
    }

    /// The tool opens a list under its button instead of running (Create Sketch).
    pub fn has_menu(&self) -> bool {
        matches!(self.action, Action::Tool("sketch.create"))
    }
}

use Tab::{Drawing as D, Manufacture as M, SheetMetal as T, Sketch as K, Solid as P};

/// Every tool of the ribbon, by tab and group; primary tools show in table order, and the
/// dropdowns list every row in table order.
pub const TOOLS: &[RibbonTool] = &[
    // SOLID > CREATE
    tool(
        P,
        "CREATE",
        Action::Tool("sketch.create"),
        Icon::SketchCreate,
        "Create Sketch",
        "Start a 2D sketch on an origin plane, a construction plane or a flat face",
    )
    .primary(),
    tool(
        P,
        "CREATE",
        Action::Extrude,
        Icon::Extrude,
        "Extrude",
        "Pull closed sketch profiles straight out into a solid",
    )
    .primary(),
    tool(
        P,
        "CREATE",
        Action::Revolve,
        Icon::Revolve,
        "Revolve",
        "Turn closed sketch profiles around a line of the same sketch",
    )
    .primary(),
    tool(
        P,
        "CREATE",
        Action::Tool("solid.hole"),
        Icon::Hole,
        "Hole",
        "Drill simple, counterbored or countersunk holes at points on a flat face",
    )
    .primary(),
    tool(
        P,
        "CREATE",
        Action::Sweep,
        Icon::Sweep,
        "Sweep",
        "Move a closed profile along a path of lines and arcs that meet smoothly",
    ),
    tool(
        P,
        "CREATE",
        Action::Loft,
        Icon::Loft,
        "Loft",
        "Join closed profiles on parallel planes into one solid, in order",
    ),
    tool(
        P,
        "CREATE",
        Action::Tool("solid.pattern_linear"),
        Icon::PatternRectangular,
        "Rectangular Pattern",
        "Repeat a feature in a row along an axis",
    )
    .sep(),
    tool(
        P,
        "CREATE",
        Action::Tool("solid.pattern_circular"),
        Icon::PatternCircular,
        "Circular Pattern",
        "Repeat a feature around an axis",
    ),
    tool(
        P,
        "CREATE",
        Action::Tool("solid.mirror"),
        Icon::MirrorBody,
        "Mirror",
        "Mirror bodies across an origin plane, as new bodies or joined",
    ),
    tool(
        P,
        "CREATE",
        Action::Tool("solid.thicken"),
        Icon::Thicken,
        "Thicken",
        "Give flat or cylindrical faces a thickness",
    )
    .sep(),
    // SOLID > MODIFY
    tool(
        P,
        "MODIFY",
        Action::PressPull,
        Icon::PressPull,
        "Press Pull",
        "Drag a profile, face or edge: extrudes, offsets the face or fillets",
    )
    .primary(),
    tool(
        P,
        "MODIFY",
        Action::Fillet,
        Icon::Fillet,
        "Fillet",
        "Round edges of a solid with a constant radius",
    )
    .primary(),
    tool(
        P,
        "MODIFY",
        Action::Chamfer,
        Icon::Chamfer,
        "Chamfer",
        "Bevel edges of a solid by an equal distance",
    )
    .primary(),
    tool(
        P,
        "MODIFY",
        Action::Tool("solid.shell"),
        Icon::Shell,
        "Shell",
        "Hollow a solid to walls of a thickness, with the faces you click removed",
    ),
    tool(
        P,
        "MODIFY",
        Action::Tool("solid.offset_face"),
        Icon::OffsetFace,
        "Offset Face",
        "Move faces along their normals; the faces around them follow",
    )
    .sep(),
    tool(
        P,
        "MODIFY",
        Action::Tool("solid.delete_face"),
        Icon::DeleteFace,
        "Delete Face",
        "Remove fillets, chamfers or holes and heal the solid",
    ),
    tool(
        P,
        "MODIFY",
        Action::Tool("solid.combine"),
        Icon::Combine,
        "Combine",
        "Join, cut or intersect bodies: click the target, then the tools",
    )
    .sep(),
    tool(
        P,
        "MODIFY",
        Action::Tool("solid.split_body"),
        Icon::SplitBody,
        "Split Body",
        "Cut bodies in two with a plane",
    ),
    tool(
        P,
        "MODIFY",
        Action::Tool("solid.split_face"),
        Icon::SplitFace,
        "Split Face",
        "Cut flat faces along a plane, keeping the shape (for draft and CAM)",
    ),
    tool(
        P,
        "MODIFY",
        Action::Tool("solid.remove"),
        Icon::Delete,
        "Remove",
        "Take the selected bodies out of the model from here on in the timeline",
    ),
    tool(
        P,
        "MODIFY",
        Action::Tool("solid.move"),
        Icon::Move,
        "Move",
        "Move or rotate bodies by typed values, or copy them",
    )
    .sep(),
    tool(
        P,
        "MODIFY",
        Action::Parameters,
        Icon::Parameters,
        "Parameters",
        "Named values and formulas that dimensions and features follow",
    )
    .sep(),
    // SOLID > SURFACE
    tool(
        P,
        "SURFACE",
        Action::Tool("surface.extrude"),
        Icon::SurfaceExtrude,
        "Surface Extrude",
        "Pull profiles or open curves straight out into open sheets, without caps",
    )
    .short("Extrude")
    .primary(),
    tool(
        P,
        "SURFACE",
        Action::Tool("surface.revolve"),
        Icon::SurfaceRevolve,
        "Surface Revolve",
        "Turn profiles or open curves about a sketch line into open sheets",
    ),
    tool(
        P,
        "SURFACE",
        Action::Tool("surface.sweep"),
        Icon::SurfaceSweep,
        "Surface Sweep",
        "Sweep profiles or open curves along a path of lines and arcs",
    ),
    tool(
        P,
        "SURFACE",
        Action::Tool("surface.patch"),
        Icon::Patch,
        "Patch",
        "Fill a closed sketch profile, or a closed flat loop of edges, with a flat face",
    )
    .primary(),
    tool(
        P,
        "SURFACE",
        Action::Tool("surface.stitch"),
        Icon::Stitch,
        "Stitch",
        "Join surfaces along their shared edges; a closed result becomes a solid",
    )
    .primary(),
    tool(
        P,
        "SURFACE",
        Action::Tool("surface.trim"),
        Icon::SurfaceTrim,
        "Trim Surface",
        "Cut a surface with a plane or a body and keep one side",
    )
    .short("Trim"),
    // SOLID > ASSEMBLE
    tool(
        P,
        "ASSEMBLE",
        Action::Tool("component.new"),
        Icon::Component,
        "New Component",
        "Group the selected body into a component of its own, or start an empty one that \
         new features go into",
    )
    .primary()
    .short("Component"),
    tool(
        P,
        "ASSEMBLE",
        Action::Tool("joint.create"),
        Icon::Joint,
        "Joint",
        "Connect two components at a face, edge or point, and say how they move",
    )
    .primary(),
    tool(
        P,
        "ASSEMBLE",
        Action::Tool("component.ground"),
        Icon::Ground,
        "Ground",
        "Fix the selected body's component in place: joints move the other side",
    ),
    tool(
        P,
        "ASSEMBLE",
        Action::Tool("joint.drag"),
        Icon::DriveJoint,
        "Drive Joint",
        "Turn or slide a joint: drag a jointed part in the viewport, or set its value",
    )
    .primary()
    .short("Drive")
    .sep(),
    tool(
        P,
        "ASSEMBLE",
        Action::Tool("motion.set_time"),
        Icon::MotionStudy,
        "Motion Study",
        "Keyframes of joint values over time: play, pause and scrub the motion",
    )
    .short("Motion"),
    tool(
        P,
        "ASSEMBLE",
        Action::Tool("inspect.interference"),
        Icon::Interference,
        "Interference",
        "Find bodies that overlap where they are placed, and the overlap volume",
    ),
    tool(
        P,
        "ASSEMBLE",
        Action::Tool("view.explode"),
        Icon::Explode,
        "Explode",
        "Show the components moved apart along the assembly's axes, with trails; the \
         model does not change",
    ),
    // SOLID > CONSTRUCT
    construct(
        "construct.plane_offset",
        "Offset Plane",
        Icon::ConstructPlaneOffset,
    )
    .primary(),
    construct(
        "construct.plane_angle",
        "Plane at Angle",
        Icon::ConstructPlaneAngle,
    ),
    construct(
        "construct.plane_midplane",
        "Midplane",
        Icon::ConstructMidplane,
    ),
    construct(
        "construct.plane_three_points",
        "Plane Through Three Points",
        Icon::ConstructPlanePoints,
    ),
    construct(
        "construct.plane_tangent",
        "Tangent Plane",
        Icon::ConstructPlaneAngle,
    ),
    construct(
        "construct.axis_cylinder",
        "Axis Through Cylinder",
        Icon::ConstructAxis,
    )
    .sep(),
    construct(
        "construct.axis_edge",
        "Axis Along Edge",
        Icon::ConstructAxis,
    ),
    construct(
        "construct.axis_two_points",
        "Axis Through Two Points",
        Icon::ConstructAxis,
    ),
    construct(
        "construct.axis_two_planes",
        "Axis Through Two Planes",
        Icon::ConstructAxis,
    ),
    construct(
        "construct.point_vertex",
        "Point at Vertex",
        Icon::ConstructPoint,
    )
    .sep(),
    construct(
        "construct.point_centre",
        "Point at Centre",
        Icon::ConstructPoint,
    ),
    construct(
        "construct.point_coordinates",
        "Point at Coordinates",
        Icon::ConstructPoint,
    ),
    // SOLID > INSPECT
    tool(
        P,
        "INSPECT",
        Action::Tool("inspect.measure"),
        Icon::Measure,
        "Measure",
        "Click two vertices, edges or faces: distance, angle, length, area",
    )
    .primary(),
    tool(
        P,
        "INSPECT",
        Action::Tool("inspect.mass"),
        Icon::Mass,
        "Mass Properties",
        "Volume, mass, centre of mass and inertia of a solid",
    ),
    // SOLID > INSERT
    tool(
        P,
        "INSERT",
        Action::ImportStl,
        Icon::ImportMesh,
        "Insert Mesh (STL, OBJ, 3MF)…",
        "Add a triangle mesh from an STL, OBJ or 3MF file, for reference",
    )
    .primary()
    .short("Insert Mesh"),
    tool(
        P,
        "INSERT",
        Action::Tool("body.import_step"),
        Icon::ImportMesh,
        "Insert STEP…",
        "Add the solids of a STEP file (AP203, AP214, AP242) as exact bodies",
    )
    .short("Insert STEP"),
    tool(
        P,
        "INSERT",
        Action::Sample,
        Icon::Body,
        "Insert Sample Part",
        "A small bracket mesh to try the viewer on",
    ),
    // SHEET METAL > CREATE
    tool(
        T,
        "CREATE",
        Action::Tool("sheet.base"),
        Icon::SheetBase,
        "Base Flange",
        "Thicken a closed sketch profile into sheet metal",
    )
    .primary(),
    tool(
        T,
        "CREATE",
        Action::Tool("sheet.flange"),
        Icon::SheetFlange,
        "Flange",
        "Bend a wall up from a straight edge of the sheet",
    )
    .primary(),
    // SHEET METAL > FLAT PATTERN
    tool(
        T,
        "FLAT PATTERN",
        Action::Tool("sheet.flat_pattern"),
        Icon::SheetUnfold,
        "Flat Pattern",
        "Unfold the sheet by its bend allowances, shown beside the part",
    )
    .primary(),
    tool(
        T,
        "FLAT PATTERN",
        Action::Tool("sheet.export_flat"),
        Icon::Export,
        "Export Flat Pattern (DXF)…",
        "Write the flat pattern's outline to a DXF file",
    )
    .primary()
    .short("Export DXF"),
    // SHEET METAL > SETUP
    tool(
        T,
        "SETUP",
        Action::Tool("sheet.rules"),
        Icon::Parameters,
        "Sheet Metal Rules",
        "Thickness, bend radius, K-factor and relief",
    )
    .primary()
    .short("Rules"),
    // SKETCH > SELECT
    sketch("SELECT", S::Select, Icon::Select, "Select").primary(),
    // SKETCH > CREATE
    sketch("CREATE", S::Line, Icon::Line, "Line").primary(),
    sketch("CREATE", S::Rectangle, Icon::Rectangle, "Rectangle").primary(),
    sketch("CREATE", S::Circle, Icon::Circle, "Circle").primary(),
    sketch("CREATE", S::Arc, Icon::Arc, "Arc").primary(),
    sketch("CREATE", S::Polygon, Icon::Polygon, "Polygon").primary(),
    sketch("CREATE", S::Spline, Icon::Spline, "Spline").primary(),
    sketch(
        "CREATE",
        S::CentreRectangle,
        Icon::RectangleCentre,
        "Centre Rectangle",
    )
    .sep(),
    sketch(
        "CREATE",
        S::ThreePointArc,
        Icon::ArcThreePoint,
        "3-Point Arc",
    ),
    sketch("CREATE", S::TangentArc, Icon::ArcTangent, "Tangent Arc"),
    sketch("CREATE", S::Slot, Icon::Slot, "Slot"),
    sketch("CREATE", S::Ellipse, Icon::Ellipse, "Ellipse"),
    sketch("CREATE", S::Project, Icon::Project, "Project").sep(),
    sketch("CREATE", S::Offset, Icon::Offset, "Offset"),
    sketch("CREATE", S::Mirror, Icon::Mirror, "Mirror"),
    sketch(
        "CREATE",
        S::RectangularPattern,
        Icon::PatternRectangular,
        "Sketch Rectangular Pattern",
    )
    .short("Rectangular Pattern"),
    sketch(
        "CREATE",
        S::CircularPattern,
        Icon::PatternCircular,
        "Sketch Circular Pattern",
    )
    .short("Circular Pattern"),
    // SKETCH > CONSTRAINTS
    tool(
        K,
        "CONSTRAINTS",
        Action::Dimension,
        Icon::Dimension,
        "Dimension",
        "Select a line, a circle, an arc or two points, then click to give it a size",
    )
    .primary(),
    constraint(
        "constraint.horizontal",
        "Horizontal",
        Icon::ConstraintHorizontal,
        "Level a line, or line up two points left to right",
    )
    .sep(),
    constraint(
        "constraint.vertical",
        "Vertical",
        Icon::ConstraintVertical,
        "Stand a line upright, or line up two points top to bottom",
    ),
    constraint(
        "constraint.coincident",
        "Coincident",
        Icon::ConstraintCoincident,
        "Join two points into one place",
    ),
    constraint(
        "constraint.point_on",
        "Point on Curve",
        Icon::ConstraintPointOn,
        "Keep a point on a line, circle or arc",
    ),
    constraint(
        "constraint.midpoint",
        "Midpoint",
        Icon::ConstraintMidpoint,
        "Keep a point halfway along a line",
    ),
    constraint(
        "constraint.parallel",
        "Parallel",
        Icon::ConstraintParallel,
        "Keep two lines pointing the same way",
    ),
    constraint(
        "constraint.perpendicular",
        "Perpendicular",
        Icon::ConstraintPerpendicular,
        "Keep two lines at a right angle",
    ),
    constraint(
        "constraint.tangent",
        "Tangent",
        Icon::ConstraintTangent,
        "Make a line or curve touch a circle or arc smoothly",
    ),
    constraint(
        "constraint.equal",
        "Equal",
        Icon::ConstraintEqual,
        "Give two lines the same length, or two circles the same radius",
    ),
    constraint(
        "constraint.fix",
        "Fix",
        Icon::ConstraintFix,
        "Pin a point where it is",
    ),
    // SKETCH > MODIFY
    sketch("MODIFY", S::Trim, Icon::Trim, "Trim").primary(),
    sketch("MODIFY", S::Extend, Icon::Extend, "Extend").primary(),
    sketch("MODIFY", S::Fillet, Icon::Fillet, "Sketch Fillet")
        .primary()
        .short("Fillet"),
    sketch("MODIFY", S::Chamfer, Icon::Chamfer, "Sketch Chamfer").short("Chamfer"),
    sketch("MODIFY", S::Split, Icon::Split, "Split"),
    tool(
        K,
        "MODIFY",
        Action::Tool("sketch.split_all"),
        Icon::Split,
        "Split All Crossings",
        "Split every curve where another crosses it, so overlapping shapes give their regions",
    ),
    tool(
        K,
        "MODIFY",
        Action::Construction,
        Icon::Construction,
        "Construction",
        "Turn the selected curves into reference geometry that profiles ignore, or back",
    )
    .sep(),
    tool(
        K,
        "MODIFY",
        Action::Delete,
        Icon::Delete,
        "Delete",
        "Remove the selected points and curves",
    ),
    // SKETCH > INSERT
    tool(
        K,
        "INSERT",
        Action::Tool("sketch.import_dxf"),
        Icon::ImportMesh,
        "Insert DXF…",
        "Draw the lines, arcs and circles of a DXF file into this sketch",
    )
    .primary()
    .short("Insert DXF"),
    tool(
        K,
        "INSERT",
        Action::Tool("sketch.export_dxf"),
        Icon::Export,
        "Export Sketch as DXF…",
        "Write this sketch's curves to a DXF file",
    ),
    // SKETCH > FINISH SKETCH
    tool(
        K,
        "FINISH SKETCH",
        Action::FinishSketch,
        Icon::SketchFinish,
        "Finish Sketch",
        "Leave the sketch and go back to the solid tools",
    )
    .primary(),
    // DRAWING > SHEET
    tool(
        D,
        "SHEET",
        Action::Tool("drawing.new_sheet"),
        Icon::New,
        "New Sheet",
        "An A3 landscape sheet at 1:1 with a title block",
    )
    .primary(),
    tool(
        D,
        "SHEET",
        Action::Tool("drawing.projection"),
        Icon::ViewRight,
        "Projection Angle",
        "Switch the sheet between third- and first-angle projection",
    )
    .primary()
    .short("Projection"),
    // DRAWING > VIEWS
    view(
        "drawing.view.front",
        "Place Front View",
        "Front",
        Icon::ViewFront,
    )
    .primary(),
    view("drawing.view.top", "Place Top View", "Top", Icon::ViewTop).primary(),
    view(
        "drawing.view.right",
        "Place Right View",
        "Right",
        Icon::ViewRight,
    )
    .primary(),
    view(
        "drawing.view.iso",
        "Place Isometric View",
        "Isometric",
        Icon::ViewIso,
    ),
    // DRAWING > DIMENSIONS
    dimension(
        "drawing.dim.horizontal",
        "Horizontal Dimension",
        "Horizontal",
        Icon::Dimension,
        "Click two vertices or edges",
    )
    .primary(),
    dimension(
        "drawing.dim.vertical",
        "Vertical Dimension",
        "Vertical",
        Icon::Dimension,
        "Click two vertices or edges",
    )
    .primary(),
    dimension(
        "drawing.dim.diameter",
        "Diameter Dimension",
        "Diameter",
        Icon::Circle,
        "Click a circle",
    )
    .primary(),
    dimension(
        "drawing.dim.aligned",
        "Aligned Dimension",
        "Aligned",
        Icon::Line,
        "Click two vertices or edges",
    ),
    dimension(
        "drawing.dim.radius",
        "Radius Dimension",
        "Radius",
        Icon::Arc,
        "Click a circle or arc",
    ),
    dimension(
        "drawing.dim.angle",
        "Angle Dimension",
        "Angle",
        Icon::ConstraintPerpendicular,
        "Click two straight edges",
    ),
    // DRAWING > EXPORT
    export("drawing.export.pdf", "Export Sheet as PDF…", "PDF").primary(),
    export("drawing.export.svg", "Export Sheet as SVG…", "SVG").primary(),
    export("drawing.export.dxf", "Export Sheet as DXF…", "DXF").primary(),
    // MANUFACTURE > SETUP
    tool(
        M,
        "SETUP",
        Action::Tool("cam.setup"),
        Icon::CamSetup,
        "Setup",
        "Stock from the model's box, part zero at the stock's top corner",
    )
    .primary(),
    tool(
        M,
        "SETUP",
        Action::Tool("cam.tool_add"),
        Icon::CamTool,
        "Add Tool",
        "A mill or drill with its feeds and speeds",
    )
    .primary(),
    // MANUFACTURE > 2D
    tool(
        M,
        "2D",
        Action::Tool("cam.op.facing"),
        Icon::CamFacing,
        "Facing",
        "Flatten the stock top down to the model's top",
    )
    .primary(),
    tool(
        M,
        "2D",
        Action::Tool("cam.op.contour"),
        Icon::CamContour,
        "2D Contour",
        "Run round a face's outline or a chain of edges",
    )
    .primary(),
    tool(
        M,
        "2D",
        Action::Tool("cam.op.pocket"),
        Icon::CamPocket,
        "2D Pocket",
        "Clear a flat-bottomed pocket: click its floor",
    )
    .primary(),
    tool(
        M,
        "2D",
        Action::Tool("cam.op.drill"),
        Icon::CamDrill,
        "Drill",
        "Drill every vertical hole of the model",
    ),
    // MANUFACTURE > ACTIONS
    tool(
        M,
        "ACTIONS",
        Action::Tool("cam.generate"),
        Icon::CamGenerate,
        "Generate Toolpaths",
        "Work every toolpath out again and show its time",
    )
    .primary()
    .short("Generate"),
    tool(
        M,
        "ACTIONS",
        Action::Tool("cam.post"),
        Icon::CamPost,
        "Post G-code",
        "Write the setup's toolpaths as G-code",
    )
    .primary()
    .short("Post"),
];

const fn construct(id: &'static str, name: &'static str, icon: Icon) -> RibbonTool {
    // The "what" line comes from `construct_ui::Kind::tip`, which the dialog shows too.
    tool(P, "CONSTRUCT", Action::Tool(id), icon, name, "")
}

const fn sketch(group: &'static str, t: S, icon: Icon, name: &'static str) -> RibbonTool {
    tool(K, group, Action::SketchTool(t), icon, name, "")
}

const fn constraint(
    id: &'static str,
    name: &'static str,
    icon: Icon,
    what: &'static str,
) -> RibbonTool {
    tool(K, "CONSTRAINTS", Action::Tool(id), icon, name, what)
}

const fn view(id: &'static str, name: &'static str, short: &'static str, icon: Icon) -> RibbonTool {
    tool(
        D,
        "VIEWS",
        Action::Tool(id),
        icon,
        name,
        "Click on the sheet to place it; views seen from the side line up with the first view",
    )
    .short(short)
}

const fn dimension(
    id: &'static str,
    name: &'static str,
    short: &'static str,
    icon: Icon,
    what: &'static str,
) -> RibbonTool {
    tool(D, "DIMENSIONS", Action::Tool(id), icon, name, what).short(short)
}

const fn export(id: &'static str, name: &'static str, short: &'static str) -> RibbonTool {
    tool(
        D,
        "EXPORT",
        Action::Tool(id),
        Icon::Export,
        name,
        "Write the shown sheet to a file",
    )
    .short(short)
}

/// The row for an action, if the ribbon has it.
#[cfg(test)]
pub fn find(action: Action) -> Option<&'static RibbonTool> {
    TOOLS.iter().find(|t| t.action == action)
}

/// The row of an `Action::Tool` id.
pub fn by_id(id: &str) -> Option<&'static RibbonTool> {
    TOOLS
        .iter()
        .find(|t| matches!(t.action, Action::Tool(i) if i == id))
}

/// The groups of a tab, in order.
pub fn groups(tab: Tab) -> impl Iterator<Item = &'static Group> {
    GROUPS.iter().filter(move |g| g.tab == tab)
}

/// The tools of a group, in table order.
pub fn tools_in(g: &Group) -> impl Iterator<Item = &'static RibbonTool> + '_ {
    TOOLS
        .iter()
        .filter(move |t| t.tab == g.tab && t.group == g.caption)
}

/// The big buttons a group shows: its primary tools, or only the first when collapsed.
pub fn buttons(g: &Group, collapsed: bool) -> Vec<&'static RibbonTool> {
    let mut v: Vec<_> = tools_in(g).filter(|t| t.primary).collect();
    if collapsed {
        v.truncate(1);
    }
    v
}

/// The small icons of a grid group (CONSTRAINTS) beside its buttons; none when collapsed.
pub fn grid(g: &Group, collapsed: bool) -> Vec<&'static RibbonTool> {
    if !g.grid || collapsed {
        return Vec::new();
    }
    tools_in(g).filter(|t| !t.primary).collect()
}

/// What the group's caption dropdown lists: every tool, or nothing for a group of one.
pub fn dropdown(g: &Group) -> Vec<&'static RibbonTool> {
    let v: Vec<_> = tools_in(g).collect();
    if v.len() > 1 { v } else { Vec::new() }
}

/// The construction tool of a `construct.*` id.
fn construct_kind(id: &str) -> Option<crate::construct_ui::Kind> {
    use crate::construct_ui::Kind;
    Kind::PLANES
        .into_iter()
        .chain(Kind::AXES)
        .chain(Kind::POINTS)
        .find(|k| k.command() == id)
}

/// The Mirror, Move or pattern tool of a `solid.*` id.
fn transform_kind(id: &str) -> Option<crate::transform_ui::TransformKind> {
    use crate::transform_ui::TransformKind as T;
    [T::Mirror, T::Move, T::PatternLinear, T::PatternCircular]
        .into_iter()
        .find(|k| k.command() == id)
}

/// The body tool of a `solid.*` id.
fn body_tool_kind(id: &str) -> Option<crate::body_tools_ui::BodyToolKind> {
    crate::body_tools_ui::BodyToolKind::ALL
        .into_iter()
        .find(|k| k.command() == id)
}

/// The surface tool of a `surface.*` id.
fn surface_kind(id: &str) -> Option<crate::surface_ui::SurfaceKind> {
    crate::surface_ui::SurfaceKind::ALL
        .into_iter()
        .find(|k| k.command() == id)
}

/// The sheet tool of a `drawing.view.*` or `drawing.dim.*` id.
fn draw_tool(id: &str) -> Option<crate::drawing_ui::DrawTool> {
    use crate::drawing_ui::DrawTool;
    Some(match id {
        "drawing.view.front" => DrawTool::PlaceView(ViewDirection::Front),
        "drawing.view.top" => DrawTool::PlaceView(ViewDirection::Top),
        "drawing.view.right" => DrawTool::PlaceView(ViewDirection::Right),
        "drawing.view.iso" => DrawTool::PlaceView(ViewDirection::Iso),
        "drawing.dim.horizontal" => DrawTool::Dimension(DimensionKind::Horizontal),
        "drawing.dim.vertical" => DrawTool::Dimension(DimensionKind::Vertical),
        "drawing.dim.aligned" => DrawTool::Dimension(DimensionKind::Aligned),
        "drawing.dim.diameter" => DrawTool::Dimension(DimensionKind::Diameter),
        "drawing.dim.radius" => DrawTool::Dimension(DimensionKind::Radius),
        "drawing.dim.angle" => DrawTool::Dimension(DimensionKind::Angular),
        _ => return None,
    })
}

/// The operation kind of a `cam.op.*` id.
fn cam_op(id: &str) -> Option<&'static str> {
    Some(match id {
        "cam.op.facing" => "facing",
        "cam.op.contour" => "contour",
        "cam.op.pocket" => "pocket",
        "cam.op.drill" => "drill",
        _ => return None,
    })
}

/// The width between two groups (`widgets::ribbon_divider`).
pub const DIVIDER: f32 = S3 + 1.0;

/// Which groups to collapse so the row fits `available` points: none if all fit, else
/// from the right, one at a time, until it fits (or all are collapsed). `full` and `one`
/// are each group's width as it is and collapsed; groups are `gap` apart.
pub fn plan(full: &[f32], one: &[f32], gap: f32, available: f32) -> Vec<bool> {
    let n = full.len();
    let mut collapsed = vec![false; n];
    let total = |c: &[bool]| -> f32 {
        let groups: f32 = c
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let w = if c { one.get(i) } else { full.get(i) };
                w.copied().unwrap_or(0.0)
            })
            .sum();
        groups + gap * n.saturating_sub(1) as f32
    };
    for i in (0..n).rev() {
        if total(&collapsed) <= available {
            break;
        }
        if let Some(c) = collapsed.get_mut(i) {
            *c = true;
        }
    }
    collapsed
}

/// A menu to open on the next frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuRequest {
    /// A group's dropdown, by caption, in the shown tab.
    Group(&'static str),
    /// The list under a tool's button (Create Sketch's planes).
    Tool(Action),
}

/// The ribbon's state.
#[derive(Default)]
pub struct RibbonUi {
    /// Open this menu on the next frame (the palette's Create Sketch, `--ribbon-menu`).
    pub request: Option<MenuRequest>,
    /// The last frame's layout, for tests: the space the groups had, and each group's
    /// caption, rectangle and whether it was collapsed.
    pub area: Option<Rect>,
    pub laid_out: Vec<(&'static str, Rect, bool)>,
}

/// What a tool looks like now.
struct ToolState {
    enabled: bool,
    active: bool,
    tip: String,
}

/// Where Create Sketch's list puts the new sketch.
#[derive(Debug, Clone, Copy)]
enum SketchOn {
    Plane(usize),
    Construct(u32),
    Face,
}

/// What the user clicked in the ribbon, done once drawing is over.
enum Clicked {
    Tool(Action),
    SketchOn(SketchOn),
}

/// Create Sketch's list: the origin planes, construction planes and a flat face.
fn sketch_on_menu(
    ui: &mut egui::Ui,
    look: Look,
    construct_planes: &[(u32, String)],
    has_solids: bool,
    long: bool,
) -> Option<SketchOn> {
    let mut on = None;
    if !long {
        widgets::caption(ui, look, "SKETCH ON");
    }
    for (i, (_, text, _)) in PLANES.iter().enumerate() {
        let item = if long {
            shortcuts::action_label(Action::CreateSketch(i))
        } else {
            (*text).to_owned()
        };
        let icon = if long {
            Icon::SketchCreate
        } else {
            Icon::Plane
        };
        if widgets::menu_item(ui, look, icon, &item, true).clicked() {
            ui.close();
            on = Some(SketchOn::Plane(i));
        }
    }
    if let Some(id) = crate::construct_ui::plane_menu(ui, look, construct_planes) {
        on = Some(SketchOn::Construct(id));
    }
    if !long {
        ui.separator();
    }
    let face = if long {
        shortcuts::action_label(Action::SketchOnFace)
    } else {
        "A flat face…".to_owned()
    };
    let icon = if long { Icon::SketchCreate } else { Icon::Body };
    let r = widgets::menu_item(ui, look, icon, &face, has_solids);
    let r = if has_solids {
        r
    } else {
        r.on_hover_text("Build a solid first")
    };
    if r.clicked() {
        ui.close();
        on = Some(SketchOn::Face);
    }
    on
}

/// A reason from `unavailable` for a tooltip: without the tool's name in front, starting
/// with a capital.
fn reason(name: &str, why: &str) -> String {
    let rest = why
        .strip_prefix(name)
        .and_then(|r| r.strip_prefix(": "))
        .unwrap_or(why);
    let mut c = rest.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

impl ChamferaApp {
    /// The workspace in use.
    pub(crate) fn workspace(&self) -> Workspace {
        let sketching = self.session.active_sketch.is_some();
        if self.drawing.active && !sketching {
            Workspace::Drawing
        } else if self.cam.active && !sketching {
            Workspace::Manufacture
        } else {
            Workspace::Design
        }
    }

    /// The tab the ribbon shows.
    pub(crate) fn ribbon_tab(&self) -> Tab {
        if self.session.active_sketch.is_some() {
            return Tab::Sketch;
        }
        match self.workspace() {
            Workspace::Design if self.sheet_metal.active => Tab::SheetMetal,
            Workspace::Design => Tab::Solid,
            Workspace::Drawing => Tab::Drawing,
            Workspace::Manufacture => Tab::Manufacture,
        }
    }

    /// Switches workspace (not while a sketch is edited).
    pub(crate) fn set_workspace(&mut self, w: Workspace) {
        if self.session.active_sketch.is_some() || self.workspace() == w {
            return;
        }
        match w {
            Workspace::Design => {
                self.drawing.active = false;
                if self.cam.active {
                    self.set_manufacture(false);
                }
            }
            Workspace::Drawing => {
                self.sheet_metal.active = false;
                self.drawing.active = true;
                if self.cam.active {
                    self.set_manufacture(false);
                }
            }
            Workspace::Manufacture => {
                self.sheet_metal.active = false;
                self.drawing.active = false;
                self.set_manufacture(true);
            }
        }
    }

    /// True while a modelling tool's dialog is open (one at a time).
    pub(crate) fn tool_open(&self) -> bool {
        let s = &self.solid;
        s.extrude.is_some()
            || s.blend.is_some()
            || s.hole.is_some()
            || s.transform.is_some()
            || s.inspect.is_some()
            || s.shell.is_some()
            || s.press_pull.is_some()
            || s.sweep.is_some()
            || s.body_tool.is_some()
            || s.surface.is_some()
            || self.construct.tool.is_some()
            || self.assembly.joint.is_some()
            || self.sheet_tool_open()
    }

    /// True when the action's tool is the one open (its button shows pressed).
    pub(crate) fn tool_active(&self, action: Action) -> bool {
        use crate::inspect_ui::InspectMode;
        use crate::press_pull_ui::PullMode;
        let s = &self.solid;
        let pull = s.press_pull.as_ref().map(|t| t.mode);
        let inspect = s.inspect.as_ref().map(|t| t.mode);
        match action {
            Action::Extrude => s.extrude.as_ref().is_some_and(|t| !t.revolve),
            Action::Revolve => s.extrude.as_ref().is_some_and(|t| t.revolve),
            Action::PressPull => pull == Some(PullMode::Auto),
            Action::Sweep => s.sweep.as_ref().is_some_and(|t| !t.loft),
            Action::Loft => s.sweep.as_ref().is_some_and(|t| t.loft),
            Action::Fillet => s.blend.as_ref().is_some_and(|t| !t.chamfer),
            Action::Chamfer => s.blend.as_ref().is_some_and(|t| t.chamfer),
            Action::Parameters => self.params.open,
            Action::SketchTool(t) => {
                self.session.active_sketch.is_some() && self.sketch.tool() == t
            }
            Action::Tool(id) => match id {
                "solid.hole" => s.hole.is_some(),
                "solid.shell" => s.shell.is_some(),
                "solid.offset_face" => pull == Some(PullMode::Offset),
                "solid.delete_face" => pull == Some(PullMode::Delete),
                "inspect.measure" => inspect == Some(InspectMode::Measure),
                "inspect.mass" => inspect == Some(InspectMode::Mass),
                "joint.create" => self.assembly.joint.is_some(),
                "component.ground" => self.ground_state().1,
                "joint.drag" => self.motion.drive.is_some(),
                "motion.set_time" => self.motion.study.is_some(),
                "inspect.interference" => self.motion.interference.is_some(),
                "view.explode" => self.motion.explode.is_some(),
                "cam.tool_add" => self.cam.tool_dialog.is_some(),
                "sheet.base" => self.sheet_metal.base.is_some(),
                "sheet.flange" => self.sheet_metal.flange.is_some(),
                "sheet.rules" => self.sheet_metal.rules.is_some(),
                _ => {
                    if let Some(k) = body_tool_kind(id) {
                        s.body_tool.as_ref().is_some_and(|t| t.kind == k)
                    } else if let Some(k) = surface_kind(id) {
                        s.surface.as_ref().is_some_and(|t| t.kind == k)
                    } else if let Some(k) = transform_kind(id) {
                        s.transform.as_ref().is_some_and(|t| t.kind == k)
                    } else if let Some(k) = construct_kind(id) {
                        self.construct.tool.as_ref().is_some_and(|t| t.kind == k)
                    } else if let Some(t) = draw_tool(id) {
                        self.drawing.tool == Some(t)
                    } else if let Some(k) = cam_op(id) {
                        self.cam.dialog.as_ref().is_some_and(|d| d.kind == k)
                    } else {
                        false
                    }
                }
            },
            _ => false,
        }
    }

    /// Why a ribbon tool with no older action of its own cannot run now, or `None`.
    pub(crate) fn tool_unavailable(&self, id: &str) -> Option<String> {
        let Some(t) = by_id(id) else {
            return Some(format!("{id} is not a tool"));
        };
        let name = t.name;
        let sketching = self.session.active_sketch.is_some();
        let has_solids = !self.session.solids().is_empty();
        let why = match t.tab {
            Tab::Sketch if !sketching => {
                format!("{name} works in a sketch: start one with Create Sketch first")
            }
            Tab::Sketch
                if id.starts_with("constraint.") && self.session.sketch_selection.is_empty() =>
            {
                format!("{name}: select sketch geometry first")
            }
            Tab::Sketch => return None,
            _ if sketching => format!("{name}: finish the sketch first"),
            Tab::Solid => match id {
                "sketch.create" => return None,
                "component.new" if self.assembly.joint.is_some() => {
                    format!("{name}: finish the joint first")
                }
                "component.new" => return None,
                "component.ground" if self.ground_state().0 == ROOT => {
                    format!("{name}: select a body of a component, or activate one, first")
                }
                "component.ground" => return None,
                "joint.create" if !has_solids => {
                    format!("{name}: build bodies in two components first")
                }
                "joint.drag" | "motion.set_time" if self.session.doc.assembly.joints.is_empty() => {
                    format!("{name}: connect two components with a Joint first")
                }
                "joint.drag" | "motion.set_time" => return None,
                "inspect.interference" if self.session.solids().len() < 2 => {
                    format!("{name}: build at least two bodies first")
                }
                "inspect.interference" => return None,
                "view.explode" if self.session.doc.assembly.components.is_empty() => {
                    format!("{name}: group bodies into components first")
                }
                "view.explode" => return None,
                "solid.remove" if has_solids && self.session.scene.selected_bodies().is_empty() => {
                    format!("{name}: select the bodies to remove first")
                }
                _ if surface_kind(id)
                    .is_some_and(|k| k.needs_sketch() && !self.has_sketch_curves()) =>
                {
                    format!("{name}: draw a sketch first")
                }
                _ if surface_kind(id).is_some_and(|k| {
                    k == crate::surface_ui::SurfaceKind::Patch && self.has_sketch_curves()
                }) || surface_kind(id).is_some_and(|k| k.needs_sketch()) =>
                {
                    if self.tool_open() {
                        format!("{name}: close the open tool first")
                    } else {
                        return None;
                    }
                }
                _ if !has_solids && construct_kind(id).is_none_or(|k| k.needs_solid()) => {
                    format!("{name}: build a solid first")
                }
                _ if self.tool_open() => format!("{name}: close the open tool first"),
                _ => return None,
            },
            Tab::SheetMetal => {
                let has_sheet = !self.session.sheets().is_empty();
                let has_profiles = self
                    .session
                    .doc
                    .sketches
                    .iter()
                    .any(|s| !s.sketch.profiles().is_empty());
                match id {
                    "sheet.base" if !has_profiles => {
                        format!("{name}: draw a closed profile in a sketch first")
                    }
                    "sheet.flange" | "sheet.flat_pattern" if !has_sheet => {
                        format!("{name}: make a base flange first")
                    }
                    "sheet.export_flat" if cfg!(target_arch = "wasm32") => {
                        format!("{name} is in the desktop app")
                    }
                    "sheet.export_flat" if self.session.flat_patterns().is_empty() => {
                        format!("{name}: make a flat pattern first")
                    }
                    _ if self.tool_open() => format!("{name}: close the open tool first"),
                    _ => return None,
                }
            }
            Tab::Drawing => match id {
                "drawing.new_sheet" => return None,
                "drawing.projection" if self.drawing_sheet().is_none() => {
                    format!("{name}: add a sheet first")
                }
                _ if id.starts_with("drawing.view.") && !has_solids => {
                    format!("{name}: build a solid first")
                }
                _ if (id.starts_with("drawing.dim.") || id.starts_with("drawing.export."))
                    && !self.drawing_has_views() =>
                {
                    format!("{name}: place a view on the sheet first")
                }
                _ => return None,
            },
            Tab::Manufacture => {
                let cam = &self.session.doc.cam;
                match id {
                    "cam.tool_add" => return None,
                    _ if (id == "cam.setup" || cam_op(id).is_some()) && !has_solids => {
                        format!("{name}: build a solid first")
                    }
                    _ if cam_op(id).is_some() && cam.tools.is_empty() => {
                        format!("{name}: add a tool first")
                    }
                    "cam.generate" | "cam.post" if cam.operations.is_empty() => {
                        format!("{name}: add an operation first")
                    }
                    _ => return None,
                }
            }
        };
        Some(why)
    }

    /// Runs a ribbon tool with no older action of its own, as its button does. Tools of
    /// another workspace switch to it first.
    pub(crate) fn run_tool(&mut self, id: &'static str) {
        let Some(t) = by_id(id) else {
            return;
        };
        match t.tab {
            Tab::Solid => {
                self.set_workspace(Workspace::Design);
                self.sheet_metal.active = false;
            }
            Tab::SheetMetal => {
                self.set_workspace(Workspace::Design);
                self.sheet_metal.active = true;
            }
            Tab::Drawing => self.set_workspace(Workspace::Drawing),
            Tab::Manufacture => self.set_workspace(Workspace::Manufacture),
            Tab::Sketch => {}
        }
        use crate::inspect_ui::InspectMode;
        use crate::press_pull_ui::PullMode;
        match id {
            "sketch.create" => self.ribbon.request = Some(MenuRequest::Tool(t.action)),
            "solid.hole" => self.open_hole_tool(),
            "solid.shell" => self.open_shell_tool(),
            "solid.offset_face" => self.open_press_pull(PullMode::Offset),
            "solid.delete_face" => self.open_press_pull(PullMode::Delete),
            "inspect.measure" => self.open_inspect(InspectMode::Measure),
            "inspect.mass" => self.open_inspect(InspectMode::Mass),
            "component.new" => self.new_component(),
            "joint.create" => self.toggle_joint(),
            "component.ground" => self.toggle_ground(),
            "joint.drag" => self.toggle_drive(),
            "motion.set_time" => self.toggle_study(),
            "inspect.interference" => self.toggle_interference(),
            "view.explode" => self.toggle_explode(),
            "sketch.import_dxf" => self.dxf_dialog(),
            "body.import_step" => self.step_dialog(),
            "sketch.export_dxf" => self.export("DXF drawing", &["dxf"], "sketch.export_dxf"),
            "sketch.split_all" => {
                self.run_sketch("sketch.split", json!({}));
            }
            "drawing.new_sheet" => self.drawing_new_sheet(),
            "drawing.projection" => self.drawing_toggle_projection(),
            "cam.setup" => self.cam_setup(),
            "cam.tool_add" => self.open_cam_tool_dialog(),
            "cam.generate" => self.generate_toolpaths(),
            "cam.post" => self.post_dialog("linuxcnc"),
            "solid.remove" => {
                self.run("solid.remove", json!({}));
            }
            "sheet.base" => self.open_base_flange(),
            "sheet.flange" => self.open_flange(),
            "sheet.flat_pattern" => self.make_flat_pattern(),
            #[cfg(not(target_arch = "wasm32"))]
            "sheet.export_flat" => self.export("DXF flat pattern", &["dxf"], "sheet.export_flat"),
            "sheet.rules" => self.open_rules(),
            _ => {
                if let Some(k) = body_tool_kind(id) {
                    self.open_body_tool(k);
                } else if let Some(k) = surface_kind(id) {
                    self.open_surface_tool(k);
                } else if let Some(kind) = id.strip_prefix("constraint.") {
                    self.run_sketch("sketch.constrain_selection", json!({ "kind": kind }));
                } else if let Some(k) = transform_kind(id) {
                    self.open_transform(k);
                } else if let Some(k) = construct_kind(id) {
                    self.open_construct(k);
                } else if let Some(tool) = draw_tool(id) {
                    self.drawing_pick_tool(tool);
                } else if let Some(f) = id.strip_prefix("drawing.export.") {
                    self.export_drawing(f);
                } else if let Some(k) = cam_op(id) {
                    self.open_cam_op(k);
                }
            }
        }
    }

    /// A click on the button of the tool that is open: tools that toggle close.
    fn tool_again(&mut self, action: Action) {
        match action {
            Action::Parameters => self.params.open = false,
            Action::Tool(id)
                if id.starts_with("inspect.")
                    || id.starts_with("drawing.view.")
                    || id.starts_with("drawing.dim.")
                    || matches!(
                        id,
                        "joint.create"
                            | "component.ground"
                            | "joint.drag"
                            | "motion.set_time"
                            | "view.explode"
                    ) =>
            {
                self.run_tool(id);
            }
            _ => {}
        }
    }

    /// A line the tooltip adds for the tool's current state.
    fn tool_note(&self, action: Action) -> Option<String> {
        match action {
            Action::Tool("drawing.projection") if self.drawing_sheet().is_some() => {
                Some(if self.drawing_third_angle() {
                    "The sheet uses third-angle projection".into()
                } else {
                    "The sheet uses first-angle projection".into()
                })
            }
            Action::Tool("component.ground") if self.ground_state().1 => {
                Some("Grounded: click to let joints move it again".into())
            }
            _ => None,
        }
    }

    /// The tool's tooltip ("Name (keys)\nwhat it does", then why it is disabled), and
    /// whether it is enabled and active.
    fn tool_state(&self, t: &RibbonTool) -> ToolState {
        let active = self.tool_active(t.action);
        let why = if active {
            None
        } else {
            self.unavailable(t.action)
        };
        let title = match shortcuts::keys_for(t.action) {
            Some(keys) => format!("{} ({keys})", t.name),
            None => t.name.to_owned(),
        };
        let what = match construct_kind(match t.action {
            Action::Tool(id) => id,
            _ => "",
        }) {
            Some(k) => k.tip(),
            None => t.what(),
        };
        let mut tip = format!("{title}\n{what}");
        if let Some(note) = self.tool_note(t.action) {
            tip.push('\n');
            tip.push_str(&note);
        }
        if let Some(why) = &why {
            tip.push('\n');
            tip.push_str(&reason(t.name, why));
        }
        ToolState {
            enabled: why.is_none(),
            active,
            tip,
        }
    }

    /// The tooltip of an action the ribbon has.
    #[cfg(test)]
    pub(crate) fn tool_tip(&self, action: Action) -> Option<String> {
        find(action).map(|t| self.tool_state(t).tip)
    }

    /// The width a group takes, as it is and collapsed.
    fn group_widths(ui: &egui::Ui, g: &Group) -> (f32, f32) {
        let widths = |tools: &[&RibbonTool]| -> f32 {
            let w: f32 = tools
                .iter()
                .map(|t| widgets::tool_width(ui, t.label(), t.has_menu()))
                .sum();
            w + S1 * tools.len().saturating_sub(1) as f32
        };
        let caption = widgets::caption_width(ui, g.caption, !dropdown(g).is_empty());
        let mut full = widths(&buttons(g, false));
        let cells = grid(g, false).len();
        if cells > 0 {
            let cols = cells.div_ceil(2) as f32;
            full += S1 + S2 + cols * GRID_CELL + (cols - 1.0) * S1;
        }
        let one = widths(&buttons(g, true));
        (full.max(caption), one.max(caption))
    }

    /// The tools of the shown tab, fitted into `available` points.
    pub(crate) fn ribbon_tools(&mut self, ui: &mut egui::Ui, available: f32) {
        let tab = self.ribbon_tab();
        let list: Vec<&'static Group> = groups(tab).collect();
        let (full, one): (Vec<f32>, Vec<f32>) =
            list.iter().map(|g| Self::group_widths(ui, g)).unzip();
        let collapsed = plan(&full, &one, DIVIDER, available);
        let mut laid_out = Vec::new();
        let mut clicked = None;
        for (i, g) in list.iter().enumerate() {
            if i > 0 {
                widgets::ribbon_divider(ui, self.look());
            }
            let c = collapsed.get(i).copied().unwrap_or(false);
            let (rect, click) = self.draw_group(ui, g, c);
            laid_out.push((g.caption, rect, c));
            clicked = clicked.or(click);
        }
        self.ribbon.laid_out = laid_out;
        self.ribbon.request = None;
        match clicked {
            Some(Clicked::Tool(a)) if self.tool_active(a) => self.tool_again(a),
            Some(Clicked::Tool(a)) => self.run_action(a),
            Some(Clicked::SketchOn(on)) => match on {
                SketchOn::Plane(i) => self.run_action(Action::CreateSketch(i)),
                SketchOn::Construct(id) => self.sketch_on_construct(id),
                SketchOn::Face => self.run_action(Action::SketchOnFace),
            },
            None => {}
        }
    }

    /// One group: its buttons, grid and caption, the dropdown and any tool's list.
    /// Returns the group's rectangle and what was clicked.
    fn draw_group(&self, ui: &mut egui::Ui, g: &Group, collapsed: bool) -> (Rect, Option<Clicked>) {
        let look = self.look();
        let shown = buttons(g, collapsed);
        let cells = grid(g, collapsed);
        let menu = dropdown(g);
        let state = |t: &RibbonTool| self.tool_state(t);
        let mut clicked = None;
        let mut menu_buttons: Vec<(Response, &RibbonTool)> = Vec::new();
        let top = ui.cursor().min.y;
        let (_, caption) = widgets::ribbon_group(ui, look, g.caption, !menu.is_empty(), |ui| {
            for t in &shown {
                let st = state(t);
                let r = widgets::tool_button(
                    ui,
                    look,
                    t.icon,
                    t.label(),
                    st.enabled || st.active,
                    st.active,
                    t.has_menu(),
                )
                .on_hover_text(st.tip);
                if t.has_menu() {
                    menu_buttons.push((r, t));
                } else if r.clicked() {
                    clicked = Some(Clicked::Tool(t.action));
                }
            }
            if !cells.is_empty() {
                ui.add_space(S2);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(S1, S1);
                    ui.add_space(1.0);
                    for row in cells.chunks(cells.len().div_ceil(2)) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = S1;
                            for t in row {
                                let st = state(t);
                                if widgets::icon_button(
                                    ui, look, t.icon, GRID_CELL, 18.0, st.enabled, st.active,
                                )
                                .on_hover_text(st.tip)
                                .clicked()
                                {
                                    clicked = Some(Clicked::Tool(t.action));
                                }
                            }
                        });
                    }
                });
            }
        });
        let request = self.ribbon.request;
        let construct_planes = self.construct_planes();
        let has_solids = !self.session.solids().is_empty();
        for (r, t) in &menu_buttons {
            let mut popup = egui::Popup::menu(r);
            if request == Some(MenuRequest::Tool(t.action)) {
                popup = popup.open_memory(Some(egui::SetOpenCommand::Bool(true)));
            }
            popup.show(|ui| {
                if let Some(on) = sketch_on_menu(ui, look, &construct_planes, has_solids, false) {
                    clicked = Some(Clicked::SketchOn(on));
                }
            });
        }
        if !menu.is_empty() {
            let mut popup = egui::Popup::menu(&caption);
            if request == Some(MenuRequest::Group(g.caption)) {
                popup = popup.open_memory(Some(egui::SetOpenCommand::Bool(true)));
            }
            popup.show(|ui| {
                ui.set_min_width(240.0);
                ui.set_max_width(300.0);
                for (i, t) in menu.iter().enumerate() {
                    if t.sep && i > 0 {
                        ui.separator();
                    }
                    if t.has_menu() {
                        if let Some(on) =
                            sketch_on_menu(ui, look, &construct_planes, has_solids, true)
                        {
                            clicked = Some(Clicked::SketchOn(on));
                        }
                        continue;
                    }
                    let st = state(t);
                    let r = widgets::menu_item(ui, look, t.icon, t.name, st.enabled || st.active);
                    if st.active {
                        widgets::menu_check(ui, look, &r);
                    }
                    if let Some(keys) = shortcuts::keys_for(t.action) {
                        widgets::menu_keys(ui, look, &r, &keys);
                    }
                    if r.on_hover_text(st.tip).clicked() {
                        ui.close();
                        clicked = Some(Clicked::Tool(t.action));
                    }
                }
            });
        }
        let rect = caption.rect.with_min_y(top);
        (rect, clicked)
    }

    /// The workspace switcher at the ribbon's left: the workspace's icon and name over a
    /// chevron, opening the list of workspaces.
    pub(crate) fn workspace_switcher(&mut self, ui: &mut egui::Ui, height: f32) {
        let look = self.look();
        let current = self.workspace();
        let sketching = self.session.active_sketch.is_some();
        let width = Workspace::ALL
            .iter()
            .map(|w| widgets::tool_width(ui, &w.name().to_uppercase(), true))
            .fold(0.0, f32::max)
            + S3;
        let (rect, r) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
        let fill = if r.hovered() || r.is_pointer_button_down_on() {
            look.c(|t| t.hover)
        } else {
            look.c(|t| t.panel_alt)
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            egui::CornerRadius::same(widgets::R_MEDIUM),
            fill,
            egui::Stroke::new(1.0, look.c(|t| t.border)),
            egui::StrokeKind::Inside,
        );
        widgets::focus_ring(ui, look, &r, widgets::R_MEDIUM);
        let icon = Rect::from_center_size(
            egui::pos2(rect.center().x, rect.min.y + height * 0.36),
            egui::Vec2::splat(28.0),
        );
        look.icon(ui, current.icon(), icon, true);
        let text = current.name().to_uppercase();
        let colour = look.c(|t| t.text);
        let galley =
            painter.layout_no_wrap(text, egui::FontId::proportional(widgets::T_SMALL), colour);
        let pos = egui::pos2(
            rect.center().x - (galley.size().x + 12.0) * 0.5,
            rect.min.y + height * 0.66,
        );
        let end = pos.x + galley.size().x;
        let mid = pos.y + galley.size().y * 0.5;
        painter.galley(pos, galley, colour);
        let chevron =
            Rect::from_center_size(egui::pos2(end + 7.0, mid + 0.5), egui::Vec2::splat(11.0));
        let (b, _) = look.icon_colours(true);
        look.icons.paint(painter, Icon::ChevronDown, chevron, b, b);
        let tip = if sketching {
            format!(
                "{} workspace\nFinish the sketch to switch workspace",
                current.name()
            )
        } else {
            format!("{} workspace\nSwitch to another workspace", current.name())
        };
        let r = r.on_hover_text(tip);
        let mut pick = None;
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(240.0);
            ui.set_max_width(300.0);
            widgets::caption(ui, look, "WORKSPACE");
            for w in Workspace::ALL {
                let item = widgets::menu_item(ui, look, w.icon(), w.name(), !sketching);
                if w == current {
                    widgets::menu_check(ui, look, &item);
                }
                if item.on_hover_text(w.tip()).clicked() {
                    ui.close();
                    pick = Some(w);
                }
            }
        });
        if let Some(w) = pick {
            self.set_workspace(w);
        }
    }

    /// The tabs of the workspace in use: Design has SOLID, and SKETCH while sketching.
    pub(crate) fn workspace_tabs(&mut self, ui: &mut egui::Ui) {
        let shown = self.ribbon_tab();
        let tabs: &[Tab] = match (self.workspace(), shown) {
            (Workspace::Design, Tab::Sketch) => &[Tab::Solid, Tab::SheetMetal, Tab::Sketch],
            (Workspace::Design, _) => &[Tab::Solid, Tab::SheetMetal],
            (Workspace::Drawing, _) => &[Tab::Drawing],
            (Workspace::Manufacture, _) => &[Tab::Manufacture],
        };
        let mut pick = None;
        for &t in tabs {
            let enabled = t == shown || shown != Tab::Sketch;
            let tip = if enabled {
                t.tip()
            } else {
                "Finish the sketch first"
            };
            if self
                .tab(ui, t.title(), t == shown, enabled)
                .on_hover_text(format!("{}\n{tip}", t.context()))
                .clicked()
            {
                pick = Some(t);
            }
        }
        match pick {
            Some(Tab::Solid) => self.sheet_metal.active = false,
            Some(Tab::SheetMetal) => self.sheet_metal.active = true,
            _ => {}
        }
    }
}

/// The side of a small icon in a grid group.
const GRID_CELL: f32 = 26.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChamferaApp;
    use eframe::App;

    /// The tools a group can reach: its buttons, its grid and its dropdown.
    fn reachable(g: &Group, collapsed: bool) -> Vec<Action> {
        let mut v: Vec<Action> = buttons(g, collapsed).iter().map(|t| t.action).collect();
        v.extend(grid(g, collapsed).iter().map(|t| t.action));
        v.extend(dropdown(g).iter().map(|t| t.action));
        v
    }

    #[test]
    fn every_tool_is_in_the_table_once_and_reachable_from_its_group() {
        // No action twice.
        for (i, a) in TOOLS.iter().enumerate() {
            for b in TOOLS.iter().skip(i + 1) {
                assert_ne!(a.action, b.action, "{} is in the table twice", a.name);
            }
        }
        // Every tool of a workspace the palette knows is in the table, except the
        // entries of Create Sketch's own list.
        let sub_items: Vec<Action> = (0..PLANES.len())
            .map(Action::CreateSketch)
            .chain([Action::SketchOnFace])
            .collect();
        for e in ChamferaApp::headless().search_results("") {
            let workspace =
                ["Solid", "Sheet Metal", "Sketch", "Drawing", "Manufacture"].contains(&e.context);
            if workspace
                && !matches!(e.action, Action::Command(_))
                && !sub_items.contains(&e.action)
            {
                let n = TOOLS.iter().filter(|t| t.action == e.action).count();
                assert_eq!(
                    n, 1,
                    "{} ({:?}) is in the ribbon {n} times",
                    e.label, e.action
                );
            }
        }
        // Every row's group exists, every group has a primary tool, and every tool is
        // reachable both when its group is shown in full and when it is collapsed.
        for t in TOOLS {
            let g = GROUPS
                .iter()
                .find(|g| g.tab == t.tab && g.caption == t.group);
            let Some(g) = g else {
                panic!("{} is in a group that does not exist: {}", t.name, t.group);
            };
            for collapsed in [false, true] {
                assert!(
                    reachable(g, collapsed).contains(&t.action),
                    "{} cannot be reached (collapsed: {collapsed})",
                    t.name
                );
            }
        }
        for g in GROUPS {
            assert!(
                tools_in(g).any(|t| t.primary),
                "{} has no big button",
                g.caption
            );
            let primaries = tools_in(g).filter(|t| t.primary).count();
            assert!(primaries <= 6, "{} has {primaries} big buttons", g.caption);
        }
    }

    #[test]
    fn every_command_of_a_tool_area_has_a_tool_or_a_reason() {
        // Commands that the ribbon reaches through another tool, or that are for picking,
        // listing, the Browser or the timeline. A newly landed tool command must get a
        // row in `TOOLS` (or a line here saying where it is used).
        const ELSEWHERE: &[&str] = &[
            "body.appearance", // View > Appearance, the display bar
            "body.import_3mf", // Insert Mesh takes STL, OBJ and 3MF
            "body.import_3mf_bytes",
            "body.import_obj",
            "body.import_obj_bytes",
            "body.import_stl_bytes", // the browser build of Insert Mesh
            "body.list",
            "body.remove", // Browser, right-click menus
            "body.rename",
            "body.visibility",
            "cam.list",
            "cam.op_add", // the 2D operation tools
            "cam.op_delete",
            "cam.op_edit", // the Manufacture tree
            "cam.show",
            "component.activate", // Browser
            "component.copy",
            "component.delete",
            "component.list",
            "component.move",
            "component.rename",
            "component.tint",
            "construct.pick",
            "drawing.add_dimension", // the dimension tools
            "drawing.add_view",      // the view tools
            "drawing.delete",
            "drawing.edit_sheet", // Projection
            "drawing.export",     // the export tools
            "drawing.export_bytes",
            "drawing.list",
            "inspect.measure_at",
            "joint.delete",
            "joint.origin_at",
            "joint.set_value",
            "solid.edge_at",
            "solid.edges",
            "solid.extrude_preview",
            "solid.move_face", // Press Pull drags faces with it
            "solid.path_at",
            "solid.profile_at",
            "sheet.rule", // the Sheet Metal Rules dialog
        ];
        let areas = [
            "solid.",
            "construct.",
            "inspect.",
            "component.",
            "joint.",
            "cam.",
            "drawing.",
            "body.",
            "sheet.",
        ];
        let covered: Vec<&str> = TOOLS
            .iter()
            .filter_map(|t| match t.action {
                Action::Tool(id) => Some(id),
                a => crate::search::command_of_action(a),
            })
            .collect();
        for c in chamfera_engine::registry() {
            if areas.iter().any(|a| c.id.starts_with(a))
                && !covered.contains(&c.id)
                && !ELSEWHERE.contains(&c.id)
            {
                panic!(
                    "{} ({}) has no ribbon tool: add it to ribbon::TOOLS",
                    c.id, c.label
                );
            }
        }
    }

    #[test]
    fn the_ribbon_and_the_palette_agree_on_names_icons_and_running() {
        let app = ChamferaApp::headless();
        for t in TOOLS {
            assert_eq!(
                shortcuts::action_label(t.action),
                t.name,
                "the palette names {:?} differently",
                t.action
            );
            assert_eq!(
                crate::search::icon_of(t.action),
                t.icon,
                "the palette shows {} with another icon",
                t.name
            );
            assert!(
                !t.what().is_empty()
                    || construct_kind(match t.action {
                        Action::Tool(id) => id,
                        _ => "",
                    })
                    .is_some(),
                "{} says nothing about what it does",
                t.name
            );
            let hits = app.search_results(t.name);
            assert!(
                hits.iter().any(|e| e.action == t.action),
                "searching \"{}\" does not find the tool",
                t.name
            );
        }
    }

    #[test]
    fn tooltips_name_the_tool_with_its_keys_and_say_why_it_is_disabled() {
        let app = ChamferaApp::headless();
        // An empty design: no profile to extrude, and E is its key.
        let extrude = app.tool_tip(Action::Extrude).unwrap_or_default();
        assert_eq!(
            extrude,
            "Extrude (E)\nPull closed sketch profiles straight out into a solid\n\
             Draw a closed profile in a sketch first"
        );
        let shell = app
            .tool_tip(Action::Tool("solid.shell"))
            .unwrap_or_default();
        assert_eq!(
            shell,
            "Shell\nHollow a solid to walls of a thickness, with the faces you click removed\n\
             Build a solid first"
        );
        // Every tooltip starts with the name, then what the tool does.
        for t in TOOLS {
            let tip = app.tool_tip(t.action).unwrap_or_default();
            let mut lines = tip.lines();
            let first = lines.next().unwrap_or_default();
            assert!(first.starts_with(t.name), "{tip}");
            assert!(lines.next().is_some_and(|l| !l.is_empty()), "{tip}");
        }
    }

    #[test]
    fn plan_collapses_groups_from_the_right_until_the_row_fits() {
        // Three groups 100, 200 and 150 wide (60, 80, 70 collapsed), 10 apart: the full row
        // is 100 + 200 + 150 + 2 * 10 = 470.
        let full = [100.0, 200.0, 150.0];
        let one = [60.0, 80.0, 70.0];
        assert_eq!(plan(&full, &one, 10.0, 470.0), [false, false, false]);
        // 469: collapsing the last gives 100 + 200 + 70 + 20 = 390.
        assert_eq!(plan(&full, &one, 10.0, 469.0), [false, false, true]);
        // 389: then the middle one: 100 + 80 + 70 + 20 = 270.
        assert_eq!(plan(&full, &one, 10.0, 389.0), [false, true, true]);
        // Too narrow for anything: all collapsed.
        assert_eq!(plan(&full, &one, 10.0, 50.0), [true, true, true]);
        assert!(plan(&[], &[], 10.0, 0.0).is_empty());
    }

    /// Runs frames of the real UI in a window `width` points wide.
    fn frames(app: &mut ChamferaApp, ctx: &egui::Context, width: f32, n: usize) {
        for k in 0..n {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 800.0),
                )),
                time: Some(k as f64 * 0.05),
                ..Default::default()
            };
            let mut f = eframe::Frame::_new_kittest();
            let mut out = ctx.run_ui(raw, |ui| app.ui(ui, &mut f));
            out.textures_delta.clear();
        }
    }

    /// Checks no group of the shown tab sticks out of the ribbon; returns how many groups
    /// were collapsed.
    fn assert_fits(app: &ChamferaApp, what: &str) -> usize {
        let Some(area) = app.ribbon.area else {
            panic!("{what}: the ribbon was not drawn");
        };
        assert!(!app.ribbon.laid_out.is_empty(), "{what}: no groups");
        for (caption, rect, _) in &app.ribbon.laid_out {
            assert!(
                rect.max.x <= area.max.x + 0.5,
                "{what}: {caption} ends at {} but the ribbon at {}",
                rect.max.x,
                area.max.x
            );
        }
        app.ribbon.laid_out.iter().filter(|g| g.2).count()
    }

    #[test]
    fn at_1100_points_no_group_overflows_in_any_tab() {
        let ctx = egui::Context::default();
        let mut app = ChamferaApp::headless();
        frames(&mut app, &ctx, 1100.0, 3);
        assert_fits(&app, "solid");
        app.run("sketch.create", json!({ "plane": "xy" }));
        frames(&mut app, &ctx, 1100.0, 3);
        assert_fits(&app, "sketch");
        app.run("sketch.finish", json!({}));
        app.set_workspace(Workspace::Drawing);
        frames(&mut app, &ctx, 1100.0, 3);
        assert_fits(&app, "drawing");
        app.set_workspace(Workspace::Manufacture);
        frames(&mut app, &ctx, 1100.0, 3);
        assert_fits(&app, "manufacture");
    }

    #[test]
    fn a_narrow_window_collapses_groups_instead_of_clipping_them() {
        let ctx = egui::Context::default();
        let mut app = ChamferaApp::headless();
        frames(&mut app, &ctx, 1920.0, 3);
        assert_eq!(
            assert_fits(&app, "solid at 1920"),
            0,
            "nothing collapses at 1920"
        );
        // At 900 the groups on the right collapse; CREATE, the first, keeps its buttons.
        frames(&mut app, &ctx, 900.0, 3);
        assert!(
            assert_fits(&app, "solid at 900") > 0,
            "some groups collapse at 900"
        );
        assert!(
            app.ribbon.laid_out.first().is_some_and(|g| !g.2),
            "CREATE collapsed before the groups after it"
        );
        // Narrower still, everything collapses but nothing is clipped.
        frames(&mut app, &ctx, 760.0, 3);
        assert!(assert_fits(&app, "solid at 760") > 0);
        app.run("sketch.create", json!({ "plane": "xy" }));
        frames(&mut app, &ctx, 900.0, 3);
        assert!(assert_fits(&app, "sketch at 900") > 0);
    }

    #[test]
    fn running_create_sketch_from_the_palette_opens_its_plane_list() {
        let ctx = egui::Context::default();
        let mut app = ChamferaApp::headless();
        frames(&mut app, &ctx, 1400.0, 2);
        app.run_action(Action::Tool("sketch.create"));
        assert_eq!(
            app.ribbon.request,
            Some(MenuRequest::Tool(Action::Tool("sketch.create")))
        );
        frames(&mut app, &ctx, 1400.0, 2);
        assert_eq!(app.ribbon.request, None);
        assert!(egui::Popup::is_any_open(&ctx), "the plane list is open");
    }

    #[test]
    fn a_drawing_tool_from_the_palette_switches_to_the_drawing_workspace() {
        let mut app = ChamferaApp::headless();
        assert_eq!(app.workspace(), Workspace::Design);
        app.run_action(Action::Tool("drawing.new_sheet"));
        assert_eq!(app.workspace(), Workspace::Drawing);
        assert_eq!(app.ribbon_tab(), Tab::Drawing);
        assert!(app.drawing_sheet().is_some(), "a sheet was added");
        app.set_workspace(Workspace::Design);
        assert_eq!(app.ribbon_tab(), Tab::Solid);
    }
}
