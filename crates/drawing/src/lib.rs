//! Layer 3: 2D engineering drawings from the 3D model.
//!
//! - [`project`]: orthographic views of solids with visible and hidden lines, computed
//!   from the exact B-rep edges (each edge sampled along its curve, every sample tested
//!   by an exact ray against the faces), cylinder silhouettes, centre marks and centre
//!   lines (`hlr.rs`).
//! - [`render_sheet`]: a [`Sheet`] laid out on paper: border, title block, views placed
//!   with first- or third-angle projection, and dimensions in ISO style (`sheet.rs`,
//!   `dims.rs`). The result is plain 2D [`Item`]s in sheet millimetres, which the UI
//!   paints and the writers export.
//! - Version 2: dimensions that follow the model through K8 references (`refs.rs`),
//!   silhouettes of cones, spheres and tori, section views with hatching (`section.rs`),
//!   detail views, notes, leaders, surface texture symbols, feature control frames,
//!   centre lines, balloons and the parts list (`annot.rs`).
//! - [`svg::write`], [`dxf::write`] and [`pdf::write`]: SVG 1.1, DXF R12 (AC1009) ASCII
//!   and a minimal hand-written PDF 1.4, from public specifications only.
//!
//! Pure: no files, no clocks, no window crates, so it builds for the browser.

pub mod annot;
mod dims;
pub mod dxf;
mod hlr;
pub mod pdf;
pub mod refs;
pub mod section;
mod sheet;
pub mod svg;
mod text;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_v2;

pub use annot::PartRow;
pub use chamfera_doc::drawing::{
    DimensionKind, Drawing, Orientation, Paper, ProjectionAngle, Sheet, SheetDimension, SheetView,
    TitleBlock, ViewDirection,
};
pub use dims::{DimensionPlan, format_value, measure, moved_offset, plan_dimension};
pub use hlr::{Basis, Curve2, ViewCurve, ViewGeometry, project, project_basis};
pub use sheet::{
    DimensionState, Owner, PlacedView, SheetGeometry, Shown, centre_line_refs, dimension_refs,
    projects_from, render_sheet, render_with_parts, scale_text, shown,
};
pub use sheet::{MARGIN, MARGIN_LEFT, TITLE_HEIGHT};
pub use text::width as text_width;

use chamfera_geom::Point2;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum DrawingError {
    #[error("{0}")]
    BadInput(String),
    #[error("the model could not be projected: {0}")]
    Kernel(String),
}

impl From<chamfera_kernel::KernelError> for DrawingError {
    fn from(e: chamfera_kernel::KernelError) -> Self {
        DrawingError::Kernel(e.to_string())
    }
}

pub type DrawingResult<T> = Result<T, DrawingError>;

/// What a 2D item is: its layer decides the line type and weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    /// Visible outlines and edges: continuous wide lines.
    Visible,
    /// Hidden edges: dashed narrow lines.
    Hidden,
    /// Centre marks, centre lines and cutting-plane lines: long-dash dot narrow lines.
    Center,
    /// Section hatching: continuous narrow lines (ISO 128-50).
    Hatch,
    /// Dimensions and annotations: continuous narrow lines, arrows and text.
    Dim,
    /// A dimension whose model reference no longer resolves: drawn where it was, in the
    /// warning colour, so that it is fixed rather than lost.
    Dangling,
    /// The border, the title block, the parts list and their text.
    Border,
}

impl Layer {
    pub const ALL: [Layer; 7] = [
        Layer::Visible,
        Layer::Hidden,
        Layer::Center,
        Layer::Hatch,
        Layer::Dim,
        Layer::Dangling,
        Layer::Border,
    ];

    /// The layer's name in DXF and SVG.
    pub fn name(self) -> &'static str {
        match self {
            Layer::Visible => "VISIBLE",
            Layer::Hidden => "HIDDEN",
            Layer::Center => "CENTER",
            Layer::Hatch => "HATCH",
            Layer::Dim => "DIM",
            Layer::Dangling => "DANGLING",
            Layer::Border => "BORDER",
        }
    }

    /// The plotted line width in mm (ISO 128-20 line groups 0.5 / 0.25).
    pub fn width(self) -> f64 {
        match self {
            Layer::Visible | Layer::Border => 0.5,
            Layer::Hidden | Layer::Center | Layer::Hatch | Layer::Dim | Layer::Dangling => 0.25,
        }
    }

    /// Dash pattern in mm (dash, gap, ...), empty for a continuous line. ISO 128-20
    /// with d = 0.25: dashed line 12d dash, 3d gap; long-dash dot 24d, 3d, 0.5d, 3d.
    pub fn dashes(self) -> &'static [f64] {
        match self {
            Layer::Hidden => &[3.0, 1.5],
            Layer::Center => &[8.0, 1.5, 0.5, 1.5],
            _ => &[],
        }
    }
}

/// How text sits on its anchor point, along its baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Centre,
    Right,
}

/// One 2D shape, in sheet mm from the sheet's lower-left corner, y up.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Line {
        a: Point2,
        b: Point2,
    },
    Circle {
        centre: Point2,
        radius: f64,
    },
    /// Counter-clockwise from `start` to `end` (radians, `start < end <= start + 2π`).
    Arc {
        centre: Point2,
        radius: f64,
        start: f64,
        end: f64,
    },
    /// `at` is on the baseline; `angle` turns the text counter-clockwise (radians).
    /// Only `Ø` and `°` beyond ASCII are used.
    Text {
        at: Point2,
        height: f64,
        angle: f64,
        align: Align,
        text: String,
    },
    /// A filled triangle (arrowheads).
    Fill {
        points: [Point2; 3],
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub layer: Layer,
    pub shape: Shape,
}

/// An export format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Svg,
    Dxf,
    Pdf,
}

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        match s {
            "svg" => Some(Format::Svg),
            "dxf" => Some(Format::Dxf),
            "pdf" => Some(Format::Pdf),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Svg => "svg",
            Format::Dxf => "dxf",
            Format::Pdf => "pdf",
        }
    }
}

/// Writes a rendered sheet in `format`.
pub fn export(sheet: &SheetGeometry, format: Format) -> Vec<u8> {
    match format {
        Format::Svg => svg::write(sheet).into_bytes(),
        Format::Dxf => dxf::write(sheet).into_bytes(),
        Format::Pdf => pdf::write(sheet),
    }
}
