---
name: chamfera-ui
description: "Use when changing Chamfera's user interface: a menu, panel, the viewport, the renderer, colours, themes, or mouse and keyboard input."
---

# The user interface

The UI (`crates/ui/src/lib.rs`) is egui on wgpu, for the desktop and the browser. It is thin: it reads the session and acts only through commands.

## Rules
- Every action runs a command through `ChamferaApp::run(id, params)`. If a button does something no command does, add the command first (`skills/chamfera-commands/SKILL.md`).
- Menus and buttons use the command's label (`label(id)`), so the UI and the command list use the same words.
- One camera path: mouse, touchpad and keys become `view.orbit`, `view.pan`, `view.zoom`, `view.fit`, `view.home` and `select.pick` in `navigate`. Never change `session.camera` directly from the UI.
- Colours come only from the theme tokens in `crates/render/src/theme.rs` (light and dark). Add a token there instead of writing a colour in UI or shader code. `text_stands_out_from_panels_in_both_themes` checks the text contrast.
- The renderer (`crates/gpu/src/lib.rs`) draws plain data from `render`: scene, camera, theme. Anything that changes the picture must be in that data, so the snapshot shows it too.
- The viewport is drawn again only when what it shows changes (`Drawn` in the UI). New state that changes the picture goes into `Drawn`.
- Default navigation: left-click selects, middle-drag pans, Shift+middle or right-drag orbits, the wheel zooms toward the cursor, F fits, H goes home, Esc clears the selection.
- Selection is a set of bodies (`Scene::selected`; `Scene::is_selected` for drawing). Shift or Ctrl+click toggles, Ctrl+Shift+click removes, a plain click on empty space clears (`select.pick` `mode`). A left drag draws a box (`crates/ui/src/sketch_select.rs`, `select.box`): to the right a window (solid outline, `select_window` tokens, fully inside), to the left a crossing (dashed, `select_crossing` tokens, touched). In a sketch the Select tool boxes points and curves when the drag starts on empty space.
- Desktop-only features (file dialogs, screenshots to disk) are behind `#[cfg(not(target_arch = "wasm32"))]`; `cargo xtask wasm` builds the browser app to check.

## Sketch mode
- `crates/ui/src/sketch_ui.rs`: the sketch toolbar, drawing tools, previews, dimension labels and inline value editing, all through `sketch.*` commands.
- Tool clicks go through `sketch_click(x, y, w, h, shift)`, which tests call with pixels (see its tests) instead of a mouse.
- Only use glyphs the bundled fonts have (Ø yes; ⌀, ✏, ✔ no): check a screenshot.

## Seeing your change
Capture and look at a screenshot before you say a UI change is done: `skills/chamfera-agent-tools/SKILL.md`.
