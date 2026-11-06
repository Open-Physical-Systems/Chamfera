//! The interface icons: SVG masters in `assets/icons/ui/`, rasterised with resvg (pure
//! Rust, so the browser build has them too) at the exact pixel size they are shown at.
//!
//! Each master uses two colours as role markers: the base colour `BASE` and the accent
//! `ACCENT`. Each role is rasterised as its own white mask and drawn tinted with theme
//! tokens (`icon` and `icon_accent`, or any pair a caller picks), so icons follow the light
//! and dark themes and no colour is baked in. The style guide is in `docs/design/ui.md`.

use egui::{Color32, ColorImage, Context, Painter, Rect, TextureHandle, TextureOptions};
use std::cell::RefCell;
use std::collections::HashMap;

/// The base role colour in the masters, drawn with the `icon` token.
const BASE: &str = "#2B313B";
/// The accent role colour in the masters, drawn with the `icon_accent` token.
const ACCENT: &str = "#E8770E";
/// Icons are never rasterised larger than this (pixels), whatever the zoom.
const MAX_PIXELS: u32 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Arc,
    ArcTangent,
    ArcThreePoint,
    Body,
    CamContour,
    CamDrill,
    CamFacing,
    CamGenerate,
    CamPocket,
    CamPost,
    CamSetup,
    CamTool,
    Chamfer,
    ChevronDown,
    ChevronRight,
    Circle,
    Close,
    Component,
    ConstraintCoincident,
    ConstraintEqual,
    ConstraintFix,
    ConstraintHorizontal,
    ConstraintMidpoint,
    ConstraintParallel,
    ConstraintPerpendicular,
    ConstraintPointOn,
    ConstraintTangent,
    ConstraintVertical,
    ConstructAxis,
    ConstructMidplane,
    ConstructPlaneAngle,
    ConstructPlaneOffset,
    ConstructPlanePoints,
    ConstructPoint,
    Construction,
    Delete,
    Dimension,
    Edit,
    Ellipse,
    Error,
    Export,
    Extend,
    Extrude,
    Eye,
    EyeOff,
    Fillet,
    Fit,
    Ground,
    Hole,
    Home,
    ImportMesh,
    Joint,
    Line,
    Mass,
    Measure,
    Loft,
    Mirror,
    MirrorBody,
    Move,
    New,
    Offset,
    Open,
    Origin,
    Parameters,
    PatternCircular,
    PatternRectangular,
    Plane,
    Polygon,
    Project,
    Rectangle,
    RectangleCentre,
    Redo,
    Revolve,
    Save,
    Select,
    SheetBase,
    SheetFlange,
    SheetUnfold,
    Shell,
    Sketch,
    SketchCreate,
    SketchFinish,
    Slot,
    Split,
    Spline,
    Sweep,
    Trim,
    Undo,
    ViewFront,
    ViewIso,
    ViewRight,
    ViewTop,
    Warning,
}

impl Icon {
    #[cfg(test)]
    pub const ALL: [Icon; 93] = [
        Icon::Arc,
        Icon::ArcTangent,
        Icon::ArcThreePoint,
        Icon::Body,
        Icon::CamContour,
        Icon::CamDrill,
        Icon::CamFacing,
        Icon::CamGenerate,
        Icon::CamPocket,
        Icon::CamPost,
        Icon::CamSetup,
        Icon::CamTool,
        Icon::Chamfer,
        Icon::ChevronDown,
        Icon::ChevronRight,
        Icon::Circle,
        Icon::Close,
        Icon::Component,
        Icon::ConstraintCoincident,
        Icon::ConstraintEqual,
        Icon::ConstraintFix,
        Icon::ConstraintHorizontal,
        Icon::ConstraintMidpoint,
        Icon::ConstraintParallel,
        Icon::ConstraintPerpendicular,
        Icon::ConstraintPointOn,
        Icon::ConstraintTangent,
        Icon::ConstraintVertical,
        Icon::ConstructAxis,
        Icon::ConstructMidplane,
        Icon::ConstructPlaneAngle,
        Icon::ConstructPlaneOffset,
        Icon::ConstructPlanePoints,
        Icon::ConstructPoint,
        Icon::Construction,
        Icon::Delete,
        Icon::Dimension,
        Icon::Edit,
        Icon::Ellipse,
        Icon::Error,
        Icon::Export,
        Icon::Extend,
        Icon::Extrude,
        Icon::Eye,
        Icon::EyeOff,
        Icon::Fillet,
        Icon::Fit,
        Icon::Ground,
        Icon::Hole,
        Icon::Home,
        Icon::ImportMesh,
        Icon::Joint,
        Icon::Line,
        Icon::Mass,
        Icon::Measure,
        Icon::Loft,
        Icon::Mirror,
        Icon::MirrorBody,
        Icon::Move,
        Icon::New,
        Icon::Offset,
        Icon::Open,
        Icon::Origin,
        Icon::Parameters,
        Icon::PatternCircular,
        Icon::PatternRectangular,
        Icon::Plane,
        Icon::Polygon,
        Icon::Project,
        Icon::Rectangle,
        Icon::RectangleCentre,
        Icon::Redo,
        Icon::Revolve,
        Icon::Save,
        Icon::Select,
        Icon::SheetBase,
        Icon::SheetFlange,
        Icon::SheetUnfold,
        Icon::Shell,
        Icon::Sketch,
        Icon::SketchCreate,
        Icon::SketchFinish,
        Icon::Slot,
        Icon::Split,
        Icon::Spline,
        Icon::Sweep,
        Icon::Trim,
        Icon::Undo,
        Icon::ViewFront,
        Icon::ViewIso,
        Icon::ViewRight,
        Icon::ViewTop,
        Icon::Warning,
    ];

    fn svg(self) -> &'static str {
        macro_rules! master {
            ($name:literal) => {
                include_str!(concat!("../../../assets/icons/ui/", $name, ".svg"))
            };
        }
        match self {
            Icon::Arc => master!("arc"),
            Icon::Body => master!("body"),
            Icon::ChevronDown => master!("chevron_down"),
            Icon::ChevronRight => master!("chevron_right"),
            Icon::Circle => master!("circle"),
            Icon::Close => master!("close"),
            Icon::ConstraintCoincident => master!("constraint_coincident"),
            Icon::ConstraintEqual => master!("constraint_equal"),
            Icon::ConstraintFix => master!("constraint_fix"),
            Icon::ConstraintHorizontal => master!("constraint_horizontal"),
            Icon::ConstraintMidpoint => master!("constraint_midpoint"),
            Icon::ConstraintParallel => master!("constraint_parallel"),
            Icon::ConstraintPerpendicular => master!("constraint_perpendicular"),
            Icon::ConstraintPointOn => master!("constraint_point_on"),
            Icon::ConstraintTangent => master!("constraint_tangent"),
            Icon::ConstraintVertical => master!("constraint_vertical"),
            Icon::ConstructAxis => master!("construct_axis"),
            Icon::ConstructMidplane => master!("construct_midplane"),
            Icon::ConstructPlaneAngle => master!("construct_plane_angle"),
            Icon::ConstructPlaneOffset => master!("construct_plane_offset"),
            Icon::ConstructPlanePoints => master!("construct_plane_points"),
            Icon::ConstructPoint => master!("construct_point"),
            Icon::Construction => master!("construction"),
            Icon::Delete => master!("delete"),
            Icon::Dimension => master!("dimension"),
            Icon::Edit => master!("edit"),
            Icon::Ellipse => master!("ellipse"),
            Icon::Error => master!("error"),
            Icon::Export => master!("export"),
            Icon::Extrude => master!("extrude"),
            Icon::Eye => master!("eye"),
            Icon::EyeOff => master!("eye_off"),
            Icon::Fit => master!("fit"),
            Icon::Hole => master!("hole"),
            Icon::Home => master!("home"),
            Icon::ImportMesh => master!("import_mesh"),
            Icon::Line => master!("line"),
            Icon::Loft => master!("loft"),
            Icon::Sweep => master!("sweep"),
            Icon::Mass => master!("mass"),
            Icon::Measure => master!("measure"),
            Icon::New => master!("new"),
            Icon::Open => master!("open"),
            Icon::Origin => master!("origin"),
            Icon::Parameters => master!("parameters"),
            Icon::Plane => master!("plane"),
            Icon::Polygon => master!("polygon"),
            Icon::Project => master!("project"),
            Icon::Rectangle => master!("rectangle"),
            Icon::Redo => master!("redo"),
            Icon::Revolve => master!("revolve"),
            Icon::Save => master!("save"),
            Icon::Select => master!("select"),
            Icon::Slot => master!("slot"),
            Icon::Shell => master!("shell"),
            Icon::SheetBase => master!("sheet_base"),
            Icon::SheetFlange => master!("sheet_flange"),
            Icon::SheetUnfold => master!("sheet_unfold"),
            Icon::Sketch => master!("sketch"),
            Icon::SketchCreate => master!("sketch_create"),
            Icon::SketchFinish => master!("sketch_finish"),
            Icon::Undo => master!("undo"),
            Icon::ViewFront => master!("view_front"),
            Icon::ViewIso => master!("view_iso"),
            Icon::ViewRight => master!("view_right"),
            Icon::ViewTop => master!("view_top"),
            Icon::ArcTangent => master!("arc_tangent"),
            Icon::ArcThreePoint => master!("arc_three_point"),
            Icon::Chamfer => master!("chamfer"),
            Icon::Extend => master!("extend"),
            Icon::Fillet => master!("fillet"),
            Icon::Mirror => master!("mirror"),
            Icon::MirrorBody => master!("mirror_body"),
            Icon::Move => master!("move"),
            Icon::Offset => master!("offset"),
            Icon::PatternCircular => master!("pattern_circular"),
            Icon::PatternRectangular => master!("pattern_rectangular"),
            Icon::RectangleCentre => master!("rectangle_centre"),
            Icon::Split => master!("split"),
            Icon::Spline => master!("spline"),
            Icon::Trim => master!("trim"),
            Icon::Warning => master!("warning"),
            Icon::CamContour => master!("cam_contour"),
            Icon::CamDrill => master!("cam_drill"),
            Icon::CamFacing => master!("cam_facing"),
            Icon::CamGenerate => master!("cam_generate"),
            Icon::CamPocket => master!("cam_pocket"),
            Icon::CamPost => master!("cam_post"),
            Icon::CamSetup => master!("cam_setup"),
            Icon::CamTool => master!("cam_tool"),
            Icon::Component => master!("component"),
            Icon::Ground => master!("ground"),
            Icon::Joint => master!("joint"),
        }
    }
}

/// The two masks of one icon at one pixel size.
struct Masks {
    base: TextureHandle,
    accent: Option<TextureHandle>,
}

/// Rasterised icons, made on first use for each (icon, pixel size).
#[derive(Default)]
pub struct Icons {
    cache: RefCell<HashMap<(Icon, u32), Option<Masks>>>,
    brand: RefCell<HashMap<u32, Option<TextureHandle>>>,
}

/// The master with one colour role kept (as white) and the other removed.
fn layer(svg: &str, keep_accent: bool) -> String {
    let (keep, drop) = if keep_accent {
        (ACCENT, BASE)
    } else {
        (BASE, ACCENT)
    };
    svg.replace(drop, "none").replace(keep, "#FFFFFF")
}

/// Rasterises an SVG to a square image `px` pixels wide.
fn rasterise(svg: &str, px: u32) -> Option<ColorImage> {
    let px = px.clamp(1, MAX_PIXELS);
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px)?;
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height()).max(1.0);
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let side = px as usize;
    Some(ColorImage::from_rgba_premultiplied(
        [side, side],
        pixmap.data(),
    ))
}

/// True when any pixel is not fully transparent.
fn has_ink(image: &ColorImage) -> bool {
    image.pixels.iter().any(|p| p.a() > 0)
}

/// The rectangle an icon is drawn into, on whole pixels so it stays sharp, and its size
/// in pixels.
fn snap(rect: Rect, ppp: f32) -> (Rect, u32) {
    let ppp = if ppp.is_finite() && ppp > 0.0 {
        ppp
    } else {
        1.0
    };
    let px = (rect.width().min(rect.height()) * ppp)
        .round()
        .clamp(1.0, MAX_PIXELS as f32);
    let min = ((rect.center().to_vec2() * ppp) - egui::Vec2::splat(px * 0.5)).round() / ppp;
    let r = Rect::from_min_size(min.to_pos2(), egui::Vec2::splat(px / ppp));
    (r, px as u32)
}

const FULL_UV: Rect = Rect {
    min: egui::pos2(0.0, 0.0),
    max: egui::pos2(1.0, 1.0),
};

impl Icons {
    fn ensure(&self, ctx: &Context, icon: Icon, px: u32) {
        let mut cache = self.cache.borrow_mut();
        cache.entry((icon, px)).or_insert_with(|| {
            let base = rasterise(&layer(icon.svg(), false), px)?;
            let accent = rasterise(&layer(icon.svg(), true), px).filter(has_ink);
            let name = format!("icon {icon:?} {px}");
            Some(Masks {
                base: ctx.load_texture(format!("{name} base"), base, TextureOptions::LINEAR),
                accent: accent
                    .map(|a| ctx.load_texture(format!("{name} accent"), a, TextureOptions::LINEAR)),
            })
        });
    }

    /// Draws `icon` into the square `rect`: the base role in `base`, the accent in `accent`.
    pub fn paint(&self, painter: &Painter, icon: Icon, rect: Rect, base: Color32, accent: Color32) {
        let ctx = painter.ctx();
        let (r, px) = snap(rect, ctx.pixels_per_point());
        self.ensure(ctx, icon, px);
        let cache = self.cache.borrow();
        let Some(Some(m)) = cache.get(&(icon, px)) else {
            return;
        };
        painter.image(m.base.id(), r, FULL_UV, base);
        if let Some(a) = &m.accent {
            painter.image(a.id(), r, FULL_UV, accent);
        }
    }

    /// Draws the Chamfera app icon, in its own colours, into the square `rect`.
    pub fn paint_brand(&self, painter: &Painter, rect: Rect) {
        let ctx = painter.ctx();
        let (r, px) = snap(rect, ctx.pixels_per_point());
        let mut brand = self.brand.borrow_mut();
        let texture = brand.entry(px).or_insert_with(|| {
            let image = rasterise(include_str!("../../../assets/icons/chamfera.svg"), px)?;
            Some(ctx.load_texture(format!("brand {px}"), image, TextureOptions::LINEAR))
        });
        if let Some(t) = texture {
            painter.image(t.id(), r, FULL_UV, Color32::WHITE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_parses_and_has_base_ink_at_small_and_large_sizes() {
        for icon in Icon::ALL {
            for px in [16, 24, 48] {
                let base = rasterise(&layer(icon.svg(), false), px);
                assert!(base.as_ref().is_some_and(has_ink), "{icon:?} at {px} px");
            }
        }
    }

    #[test]
    fn masters_are_on_the_grid_and_use_only_the_two_role_colours() {
        for icon in Icon::ALL {
            let svg = icon.svg();
            assert!(svg.contains("viewBox=\"0 0 24 24\""), "{icon:?}");
            assert!(svg.contains("stroke-width=\"1.75\""), "{icon:?}");
            for (i, _) in svg.match_indices('#') {
                let hex = svg.get(i..i + 7).unwrap_or("");
                assert!(hex == BASE || hex == ACCENT, "{icon:?} uses {hex}");
            }
        }
    }

    #[test]
    fn the_masks_split_the_colour_roles() {
        // The rectangle: a base outline from (4.5, 6.5) to (19.5, 17.5) and accent dots on
        // two corners. Pixel (4, 17) is on the bottom-left dot; pixel (12, 17) is on the
        // bottom edge, away from the dots.
        let accent = rasterise(&layer(Icon::Rectangle.svg(), true), 24).unwrap();
        let base = rasterise(&layer(Icon::Rectangle.svg(), false), 24).unwrap();
        let alpha = |img: &ColorImage, x: usize, y: usize| img.pixels[y * 24 + x].a();
        assert!(alpha(&accent, 4, 17) > 200);
        assert_eq!(alpha(&accent, 12, 17), 0);
        assert!(alpha(&base, 12, 17) > 100);
        // Close has no accent part at all.
        let none = rasterise(&layer(Icon::Close.svg(), true), 24).unwrap();
        assert!(!has_ink(&none));
    }

    #[test]
    fn icons_land_on_whole_pixels() {
        let (r, px) = snap(
            Rect::from_min_size(egui::pos2(10.3, 4.6), egui::vec2(16.0, 16.0)),
            2.0,
        );
        assert_eq!(px, 32);
        assert_eq!(r.min.x * 2.0, (r.min.x * 2.0).round());
        assert_eq!(r.min.y * 2.0, (r.min.y * 2.0).round());
        assert_eq!(r.width(), 16.0);
    }

    /// Writes a contact sheet of every icon, light theme above dark, at 24 and 48 px:
    /// `CHAMFERA_ICON_SHEET=/tmp/icons.png cargo test -p chamfera-ui icon_sheet -- --ignored`.
    #[test]
    #[ignore]
    fn icon_sheet() {
        let Ok(path) = std::env::var("CHAMFERA_ICON_SHEET") else {
            return;
        };
        let (cols, cell) = (8usize, 84usize);
        let rows = Icon::ALL.len().div_ceil(cols);
        let (w, h) = (cols * cell, rows * cell * 2);
        let mut img = vec![0u8; w * h * 4];
        let themes = [
            chamfera_render::Theme::light(),
            chamfera_render::Theme::dark(),
        ];
        for (half, theme) in themes.iter().enumerate() {
            let y0 = half * rows * cell;
            for y in y0..y0 + rows * cell {
                for x in 0..w {
                    let i = (y * w + x) * 4;
                    let bg = theme.chrome;
                    img[i..i + 4].copy_from_slice(&[bg.0, bg.1, bg.2, 255]);
                }
            }
            for (k, icon) in Icon::ALL.iter().enumerate() {
                let (cx, cy) = ((k % cols) * cell, y0 + (k / cols) * cell);
                for (px, ox, oy) in [(24usize, 6usize, 30usize), (48, 32, 18)] {
                    for (accent, tint) in [(false, theme.icon), (true, theme.icon_accent)] {
                        let m = rasterise(&layer(icon.svg(), accent), px as u32).unwrap();
                        for y in 0..px {
                            for x in 0..px {
                                let a = f32::from(m.pixels[y * px + x].a()) / 255.0;
                                let i = ((cy + oy + y) * w + cx + ox + x) * 4;
                                for (c, t) in [tint.0, tint.1, tint.2].into_iter().enumerate() {
                                    let d = f32::from(img[i + c]);
                                    img[i + c] = (d + (f32::from(t) - d) * a).round() as u8;
                                }
                            }
                        }
                    }
                }
            }
        }
        let file = std::fs::File::create(path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.write_header().unwrap().write_image_data(&img).unwrap();
    }
}
