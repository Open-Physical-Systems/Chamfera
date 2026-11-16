//! The bill of materials: one row per part (a component that is not a linked copy and
//! shows bodies), its quantity counting every linked copy of it wherever in the
//! component tree the copy sits, its material, volume and, when its bodies have a
//! density, its mass.
//!
//! A linked copy shows its original's own bodies (not those of the original's child
//! components), so a part's quantity is the number of its instances: the original plus
//! its copies. Bodies of the design itself (the root component) are one row too.

use crate::Session;
use chamfera_doc::ROOT;

/// One row of the bill of materials.
#[derive(Debug, Clone, PartialEq)]
pub struct BomRow {
    /// 1, 2, ... in component order.
    pub item: u32,
    pub part: String,
    /// The original component (`ROOT` for the design's own bodies).
    pub component: u32,
    /// The original and its linked copies.
    pub instances: Vec<u32>,
    pub quantity: u32,
    /// Bodies in one instance.
    pub bodies: usize,
    /// Volume of one instance, mm³ (`None` when a body has no volume, such as a surface).
    pub volume: Option<f64>,
    /// The named appearance of its first body that has one, else empty.
    pub material: String,
    /// Density in g/cm³, when the part's bodies have one.
    pub density: Option<f64>,
    /// Mass of one instance in g, when the density is known.
    pub mass: Option<f64>,
}

impl BomRow {
    /// Volume of all instances, mm³.
    pub fn total_volume(&self) -> Option<f64> {
        self.volume.map(|v| v * f64::from(self.quantity))
    }
}

/// The bill of materials of the design as built.
pub fn bom(s: &Session) -> Vec<BomRow> {
    let asm = &s.doc.assembly;
    let mut parts: Vec<(u32, String)> = Vec::new();
    if s.solids().iter().any(|x| x.component == ROOT) {
        let name = s
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or_else(|| "Design".to_owned(), |n| n.to_string_lossy().into_owned());
        parts.push((ROOT, name));
    }
    parts.extend(
        asm.components
            .iter()
            .filter(|c| c.occurrence_of.is_none())
            .filter(|c| s.solids().iter().any(|x| x.component == c.id))
            .map(|c| (c.id, c.name.clone())),
    );
    let mut rows = Vec::with_capacity(parts.len());
    for (item, (id, part)) in parts.into_iter().enumerate() {
        let mut instances = vec![id];
        instances.extend(
            asm.components
                .iter()
                .filter(|o| id != ROOT && o.occurrence_of == Some(id))
                .map(|o| o.id),
        );
        let solids: Vec<&crate::solids::Solid> =
            s.solids().iter().filter(|x| x.component == id).collect();
        let volume = solids
            .iter()
            .map(|x| x.body.volume().ok().filter(|v| v.is_finite() && *v > 0.0))
            .sum::<Option<f64>>();
        let material = solids
            .iter()
            .find_map(|x| {
                s.doc
                    .appearances
                    .get(&x.feature)
                    .and_then(|a| a.preset.clone())
            })
            .unwrap_or_default();
        // No body stores a physical density yet; when one does, mass follows from it.
        let density: Option<f64> = None;
        let mass = volume.zip(density).map(|(v, d)| v * d / 1000.0);
        rows.push(BomRow {
            item: u32::try_from(item + 1).unwrap_or(u32::MAX),
            part,
            component: id,
            quantity: u32::try_from(instances.len()).unwrap_or(u32::MAX),
            instances,
            bodies: solids.len(),
            volume,
            material,
            density,
            mass,
        });
    }
    rows
}

/// The CSV header of [`bom_csv`].
pub const BOM_HEADER: [&str; 7] = [
    "item",
    "part",
    "quantity",
    "material",
    "volume_mm3",
    "density_g_cm3",
    "mass_g",
];

/// The bill of materials as CSV (RFC 4180, CRLF line ends): one line per row.
pub fn bom_csv(rows: &[BomRow]) -> String {
    let num = |v: Option<f64>| v.map_or_else(String::new, |v| format!("{v:.3}"));
    let table: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.item.to_string(),
                r.part.clone(),
                r.quantity.to_string(),
                r.material.clone(),
                num(r.volume),
                num(r.density),
                num(r.mass),
            ]
        })
        .collect();
    chamfera_io::csv::write_table(&BOM_HEADER, &[0, 2, 4, 5, 6], &table)
}
