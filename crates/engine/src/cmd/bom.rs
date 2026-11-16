//! `bom.*`: the bill of materials.

use super::CommandSpec;
use crate::bom::{BomRow, bom, bom_csv};
use crate::{CmdResult, Session, params};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "bom.export_csv",
        label: "Export Bill of Materials",
        params: "path: string (optional) - the CSV file to write; without it the CSV comes back as `csv`\n\
                 Columns: item, part, quantity, material, volume_mm3, density_g_cm3, mass_g (one instance each).",
        mutates: false,
        run: export_csv,
    },
    CommandSpec {
        id: "bom.list",
        label: "Bill of Materials",
        params: "(none) - one row per part (a component that is not a linked copy and has bodies, and the design's own bodies): item, part, quantity counting its linked copies, material, volume, density and mass when known",
        mutates: false,
        run: list,
    },
];

fn row_json(r: &BomRow) -> Value {
    json!({
        "item": r.item,
        "part": r.part,
        "component": r.component,
        "instances": r.instances,
        "quantity": r.quantity,
        "bodies": r.bodies,
        "volume": r.volume,
        "total_volume": r.total_volume(),
        "material": r.material,
        "density": r.density,
        "mass": r.mass,
    })
}

fn list(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let rows = bom(s);
    let parts: u64 = rows.iter().map(|r| u64::from(r.quantity)).sum();
    Ok(json!({
        "rows": rows.iter().map(row_json).collect::<Vec<_>>(),
        "parts": parts,
    }))
}

fn export_csv(s: &mut Session, p: &Value) -> CmdResult {
    let rows = bom(s);
    let text = bom_csv(&rows);
    match params::opt_str(p, "path")? {
        Some(path) => {
            std::fs::write(path, text.as_bytes()).map_err(chamfera_io::IoError::from)?;
            Ok(json!({ "path": path, "rows": rows.len() }))
        }
        None => Ok(json!({ "csv": text, "rows": rows.len() })),
    }
}
