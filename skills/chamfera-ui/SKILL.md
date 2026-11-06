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
- Keyboard shortcuts live in one table, `SHORTCUTS` in `crates/ui/src/shortcuts.rs`: tooltips (`shortcuts::tip`), the search palette and Help > Keyboard Shortcuts read it, and `no_two_shortcuts_share_a_key` checks it. Shortcuts never fire while a text box has the keyboard.
- The search palette (`crates/ui/src/search.rs`, Ctrl+Space then F) lists every UI action and every registry command; a new tool gets an `Action` there so the palette and its key run it as its button does.
- Right-click menus (`crates/ui/src/context_menu.rs`): a right click (no drag) opens the menu for what is under the cursor; items are `Entry` rows whose `Act` runs a command or `run_action`, the same path as the ribbon. The Browser reuses them (`body_entries`, `show_entries`). Items that cannot run are shown disabled with the reason.
- The selection filter (bottom-left of the viewport) goes to `select.pick` and `sketch.select` as `filter`; hover pre-highlight uses `Session::pick_filtered` only when the pointer or view moved, drawn in the `hover_highlight` token.
- Default navigation: left-click selects, middle-drag pans, Shift+middle or right-drag orbits, the wheel zooms toward the cursor, F fits, H goes home, Esc clears the selection.
- Navigation presets (`crates/ui/src/navigation.rs`): `drag_for` maps a button and modifiers to orbit, pan or box select once per drag, for the preset in Preferences (Chamfera default, Fusion-, Inventor-, SolidWorks-style, left-drag orbit for touchpads). Zoom direction, zoom to cursor, orbit about the cursor (`view.point_at` then `view.orbit` with `pivot`) and sensitivity live there too. In a sketch the left button stays with the sketch tools.
- Preferences (`prefs.rs` data and file, `prefs_ui.rs` window, Ctrl+, or File > Preferences) are per user: the platform config folder (`~/.config/chamfera/preferences.json` on Linux), local storage in the browser; never the design. Reading a broken file gives defaults field by field. Tests use `ChamferaApp::headless()`, which never reads or writes the file.
- Units: show lengths and angles with `self.units()` (`chamfera_doc::DisplayUnits`: `length`, `area`, `volume`, `angle`), never a hard-coded `mm` or `{:.2}`. A plain typed number is in the design's unit (`design.units`); parse typed values with `expr::evaluate_as_in`.
- Selection is a set of bodies (`Scene::selected`; `Scene::is_selected` for drawing). Shift or Ctrl+click toggles, Ctrl+Shift+click removes, a plain click on empty space clears (`select.pick` `mode`). A left drag draws a box (`crates/ui/src/sketch_select.rs`, `select.box`): to the right a window (solid outline, `select_window` tokens, fully inside), to the left a crossing (dashed, `select_crossing` tokens, touched). In a sketch the Select tool boxes points and curves when the drag starts on empty space.
- Desktop-only features (file dialogs, screenshots to disk) are behind `#[cfg(not(target_arch = "wasm32"))]`; `cargo xtask wasm` builds the browser app to check.

## Sketch mode
- `crates/ui/src/sketch_ui.rs`: the sketch toolbar, drawing tools, previews, dimension labels and inline value editing, all through `sketch.*` commands.
- Tool clicks go through `sketch_click(x, y, w, h, shift)`, which tests call with pixels (see its tests) instead of a mouse.
- Precise input: clicks snap through `Sketch::snap` (`crates/sketch/src/snap.rs`, pure geometry) and the snap's constraints are added with one `sketch.constrain {"constraints": [...]}`; Shift locks a segment horizontal or vertical (Shift+Alt: 15 degree steps); typed numbers fill the heads-up fields (Tab is taken in `raw_input_hook` so it moves between fields, not the focus). Tests that drive real pointer and key events are in `crates/ui/src/sketch_input_tests.rs`.
- Only use glyphs the bundled fonts have (Ø yes; ⌀, ✏, ✔ no): check a screenshot.

## Seeing your change
Capture and look at a screenshot before you say a UI change is done: `skills/chamfera-agent-tools/SKILL.md`.
