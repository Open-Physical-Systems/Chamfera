---
name: chamfera-ground-rules
description: "Use before adding any dependency, icon, image, font, reference data or product wording to Chamfera, or when unsure whether something may be copied, read or named. Covers independence from commercial CAD, copyleft code and the product name."
---

# Independence, licences and the name

## Independent
- Behaviour comes from public documentation, general CAD conventions, textbooks and user feedback. File formats come from public specifications (ISO 10303 for STEP, the 3MF spec, ...).
- A "Fusion-style workflow" is fine: workspaces, a timeline, sketches, features. Copying Autodesk's (or any vendor's) icons, UI text, help, tooltips, templates, post processors or file internals is not.
- Never drive, script or extract data from commercial CAD (Fusion, SolidWorks, Onshape, ...) for this project.
- Command ids are our own (`skills/chamfera-commands/SKILL.md`).
- In public text say "familiar to Fusion users", never "Fusion clone". Trademarks are named only to describe compatibility.

## No copyleft code
- Do not read, copy or link OpenCASCADE, FreeCAD (incl. planegcs and its CAM), SolveSpace, CadQuery/OCP, LibreCAD, QCAD, OpenCAMLib, CAMotics, pycam, BlenderCAM. Do not even read their sources.
- Permissive projects (MIT, Apache-2.0, BSD, ISC, Zlib, BSL-1.0) may be read for reference. Our code is written by us; anything adapted is credited in the PR and in a source comment.
- `deny.toml` is the licence allow-list and `cargo xtask licences` enforces it. Never add a copyleft licence to it. A new dependency needs a reason in the commit message: what it does and why we don't write it.
- Prefer few dependencies, and pure Rust ones that build for wasm.

## Assets
- No icons, images, fonts or artwork from commercial CAD, ever. Every asset is original or openly licensed and credited.

## The name
- Chamfera everywhere: crates `chamfera-*`, binaries `chamfera` and `chamfera-cli`, `.chamfera` design files, `CHAMFERA_*` environment variables.
- Units are millimetres and radians internally; Z is up.
