//! Keyboard shortcuts: one table of actions and their default keys. The ribbon tooltips,
//! the search palette and Help > Keyboard Shortcuts all read it, and `shortcut_keys`
//! runs the single-key ones.
//!
//! Shortcuts never fire while a text box has the keyboard.

use crate::sketch_ui::Tool;
use crate::{ChamferaApp, label};
use egui::{Key, Modifiers};

/// Something a user can do from a button, a menu, the search palette or a key. Running
/// one (`ChamferaApp::run_action` in `search.rs`) does what its button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Pick a sketch tool (only while a sketch is being edited).
    SketchTool(Tool),
    /// Create a sketch on one of `sketch_ui::PLANES`, by index.
    CreateSketch(usize),
    SketchOnFace,
    FinishSketch,
    Extrude,
    Revolve,
    /// Press Pull: extrude a profile, offset a face or fillet an edge by dragging.
    PressPull,
    Sweep,
    Loft,
    Fillet,
    Chamfer,
    Parameters,
    Dimension,
    Construction,
    Delete,
    Cancel,
    Undo,
    Redo,
    Fit,
    Home,
    /// One of the standard views: front, back, left, right, top, bottom or iso.
    StandardView(&'static str),
    ToggleProjection,
    New,
    Open,
    Save,
    SaveAs,
    ExportStep,
    ExportStl,
    ImportStl,
    Sample,
    ShowShortcuts,
    Search,
    Preferences,
    /// Show or hide the Assistant panel.
    Assistant,
    /// A registry command with no tool of its own.
    Command(&'static str),
    /// A ribbon tool with no action above, by its id in `ribbon::TOOLS` (a command id
    /// such as `solid.shell`, or the ribbon's own such as `drawing.view.front`).
    Tool(&'static str),
}

/// How a shortcut is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keys {
    /// One key, with or without Ctrl (Cmd on a Mac) and Shift.
    Single { key: Key, ctrl: bool, shift: bool },
    /// Ctrl+Space, then this key (with Ctrl still held, or within about a second).
    CtrlSpaceThen(Key),
    /// Ctrl (Cmd on a Mac) and Alt with this key.
    CtrlAlt(Key),
}

impl Keys {
    const fn key(key: Key) -> Keys {
        Keys::Single {
            key,
            ctrl: false,
            shift: false,
        }
    }

    const fn ctrl(key: Key) -> Keys {
        Keys::Single {
            key,
            ctrl: true,
            shift: false,
        }
    }

    const fn ctrl_shift(key: Key) -> Keys {
        Keys::Single {
            key,
            ctrl: true,
            shift: true,
        }
    }

    /// How the keys are written in tooltips and lists: "L", "Ctrl+Shift+S", "Ctrl+Space, F".
    pub fn text(self) -> String {
        match self {
            Keys::Single { key, ctrl, shift } => {
                let mut s = String::new();
                if ctrl {
                    s.push_str("Ctrl+");
                }
                if shift {
                    s.push_str("Shift+");
                }
                s.push_str(key_name(key));
                s
            }
            Keys::CtrlSpaceThen(key) => format!("Ctrl+Space, {}", key_name(key)),
            Keys::CtrlAlt(key) => format!("Ctrl+Alt+{}", key_name(key)),
        }
    }
}

fn key_name(key: Key) -> &'static str {
    match key {
        Key::Escape => "Esc",
        Key::Delete => "Del",
        other => other.name(),
    }
}

/// Where a shortcut is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handler {
    /// `shortcut_keys` below.
    Here,
    /// The search palette's own key handling (`search.rs`).
    Search,
    /// Older code that already reads the key: `navigate` (F, H, Esc), `sketch_input`
    /// (Esc, Del) and `file_shortcuts` (Ctrl keys).
    Elsewhere,
}

/// One row of the table.
#[derive(Debug, Clone, Copy)]
pub struct Shortcut {
    pub action: Action,
    pub keys: Keys,
    /// Where the key works, for the Help list.
    pub context: &'static str,
    pub handler: Handler,
}

const fn row(action: Action, keys: Keys, context: &'static str, handler: Handler) -> Shortcut {
    Shortcut {
        action,
        keys,
        context,
        handler,
    }
}

/// Every default shortcut. Keys are unique (`no_two_shortcuts_share_a_key`).
pub const SHORTCUTS: [Shortcut; 27] = [
    row(
        Action::Search,
        Keys::CtrlSpaceThen(Key::F),
        "Anywhere",
        Handler::Search,
    ),
    row(
        Action::SketchTool(Tool::Line),
        Keys::key(Key::L),
        "Sketch",
        Handler::Here,
    ),
    row(
        Action::SketchTool(Tool::Rectangle),
        Keys::key(Key::R),
        "Sketch",
        Handler::Here,
    ),
    row(
        Action::SketchTool(Tool::Circle),
        Keys::key(Key::C),
        "Sketch",
        Handler::Here,
    ),
    row(
        Action::SketchTool(Tool::Arc),
        Keys::key(Key::A),
        "Sketch",
        Handler::Here,
    ),
    row(
        Action::SketchTool(Tool::Trim),
        Keys::key(Key::T),
        "Sketch",
        Handler::Here,
    ),
    row(
        Action::SketchTool(Tool::Offset),
        Keys::key(Key::O),
        "Sketch",
        Handler::Here,
    ),
    row(
        Action::Dimension,
        Keys::key(Key::D),
        "Sketch",
        Handler::Here,
    ),
    row(
        Action::Construction,
        Keys::key(Key::X),
        "Sketch",
        Handler::Here,
    ),
    row(Action::Extrude, Keys::key(Key::E), "Solid", Handler::Here),
    row(Action::PressPull, Keys::key(Key::Q), "Solid", Handler::Here),
    row(
        Action::Delete,
        Keys::key(Key::Delete),
        "Sketch",
        Handler::Elsewhere,
    ),
    row(
        Action::Cancel,
        Keys::key(Key::Escape),
        "Anywhere",
        Handler::Elsewhere,
    ),
    row(
        Action::Fit,
        Keys::key(Key::F),
        "Viewport",
        Handler::Elsewhere,
    ),
    row(
        Action::Home,
        Keys::key(Key::H),
        "Viewport",
        Handler::Elsewhere,
    ),
    row(
        Action::Undo,
        Keys::ctrl(Key::Z),
        "Anywhere",
        Handler::Elsewhere,
    ),
    row(
        Action::Redo,
        Keys::ctrl(Key::Y),
        "Anywhere",
        Handler::Elsewhere,
    ),
    row(
        Action::Redo,
        Keys::ctrl_shift(Key::Z),
        "Anywhere",
        Handler::Elsewhere,
    ),
    row(
        Action::New,
        Keys::ctrl(Key::N),
        "Anywhere",
        Handler::Elsewhere,
    ),
    row(
        Action::Open,
        Keys::ctrl(Key::O),
        "Anywhere",
        Handler::Elsewhere,
    ),
    row(
        Action::Save,
        Keys::ctrl(Key::S),
        "Anywhere",
        Handler::Elsewhere,
    ),
    row(
        Action::SaveAs,
        Keys::ctrl_shift(Key::S),
        "Anywhere",
        Handler::Elsewhere,
    ),
    row(
        Action::Command("version.save"),
        Keys::CtrlAlt(Key::S),
        "Anywhere",
        Handler::Elsewhere,
    ),
    row(
        Action::Command("timeline.rename"),
        Keys::key(Key::F2),
        "Timeline, Browser",
        Handler::Elsewhere,
    ),
    row(
        Action::ShowShortcuts,
        Keys::ctrl(Key::Slash),
        "Anywhere",
        Handler::Here,
    ),
    row(
        Action::Preferences,
        Keys::ctrl(Key::Comma),
        "Anywhere",
        Handler::Elsewhere,
    ),
    row(
        Action::Assistant,
        Keys::ctrl_shift(Key::A),
        "Anywhere",
        Handler::Here,
    ),
];

/// Keys kept free for tools that are planned, so nothing else takes them.
pub const RESERVED: [(Key, &str); 0] = [];

/// The first shortcut of an action, as text ("L", "Ctrl+Z").
pub fn keys_for(action: Action) -> Option<String> {
    SHORTCUTS
        .iter()
        .find(|s| s.action == action)
        .map(|s| s.keys.text())
}

/// A tooltip with the action's shortcut after its first line: "Line (L)\nDraw ...".
pub fn tip(text: &str, action: Action) -> String {
    let Some(keys) = keys_for(action) else {
        return text.to_owned();
    };
    match text.split_once('\n') {
        Some((title, rest)) => format!("{title} ({keys})\n{rest}"),
        None => format!("{text} ({keys})"),
    }
}

/// An action's name, as menus and the palette show it.
pub fn action_label(action: Action) -> String {
    match action {
        Action::SketchTool(Tool::Select) => "Select".into(),
        Action::SketchTool(Tool::Fillet) => label("sketch.fillet").into(),
        Action::SketchTool(Tool::Chamfer) => label("sketch.chamfer").into(),
        Action::SketchTool(Tool::RectangularPattern) => "Sketch Rectangular Pattern".into(),
        Action::SketchTool(Tool::CircularPattern) => "Sketch Circular Pattern".into(),
        Action::SketchTool(t) => t.name().into(),
        Action::CreateSketch(i) => match crate::sketch_ui::PLANES.get(i) {
            Some((_, text, _)) => format!("{} on {text}", label("sketch.create")),
            None => label("sketch.create").into(),
        },
        Action::SketchOnFace => label("sketch.create_on_face").into(),
        Action::FinishSketch => label("sketch.finish").into(),
        Action::Extrude => label("solid.extrude").into(),
        Action::Revolve => label("solid.revolve").into(),
        Action::PressPull => label("solid.press_pull").into(),
        Action::Sweep => label("solid.sweep").into(),
        Action::Loft => label("solid.loft").into(),
        Action::Fillet => label("solid.fillet").into(),
        Action::Chamfer => label("solid.chamfer").into(),
        Action::Parameters => "Parameters".into(),
        Action::Dimension => "Dimension".into(),
        Action::Construction => label("sketch.construction").into(),
        Action::Delete => label("sketch.remove").into(),
        Action::Cancel => "Cancel the tool or clear the selection".into(),
        Action::Undo => label("edit.undo").into(),
        Action::Redo => label("edit.redo").into(),
        Action::Fit => label("view.fit").into(),
        Action::Home => label("view.home").into(),
        Action::StandardView(v) => format!("{} View", capitalised(v)),
        Action::ToggleProjection => "Switch Perspective / Orthographic".into(),
        Action::New => label("design.new").into(),
        Action::Open => format!("{}…", label("design.open")),
        Action::Save => label("design.save").into(),
        Action::SaveAs => "Save Design As…".into(),
        Action::ExportStep => format!("{}…", label("design.export_step")),
        Action::ExportStl => format!("{}…", label("design.export_stl")),
        Action::ImportStl => "Insert Mesh (STL, OBJ, 3MF)…".into(),
        Action::Sample => label("body.sample").into(),
        Action::ShowShortcuts => "Keyboard Shortcuts".into(),
        Action::Preferences => "Preferences".into(),
        Action::Assistant => "Assistant".into(),
        Action::Search => "Search Tools and Commands".into(),
        Action::Command(id) => label(id).into(),
        Action::Tool(id) => {
            crate::ribbon::by_id(id).map_or_else(|| label(id).into(), |t| t.name.into())
        }
    }
}

fn capitalised(word: &str) -> String {
    if word == "iso" {
        return "Isometric".into();
    }
    let mut c = word.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// True when `m` is exactly Ctrl (or Cmd) as asked, Shift as asked, and no Alt.
fn modifiers_match(m: Modifiers, ctrl: bool, shift: bool) -> bool {
    (m.ctrl || m.command || m.mac_cmd) == ctrl && m.shift == shift && !m.alt
}

impl ChamferaApp {
    /// Runs the single-key shortcuts of the table (`Handler::Here`) and takes their key
    /// events, so nothing else reads them. Does nothing while a text box has the keyboard
    /// or the search palette is open.
    pub(crate) fn shortcut_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.search.open {
            return;
        }
        let mut actions: Vec<Action> = Vec::new();
        ctx.input_mut(|i| {
            i.events.retain(|e| {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = e
                else {
                    return true;
                };
                let hit = SHORTCUTS.iter().find(|s| {
                    s.handler == Handler::Here
                        && matches!(s.keys, Keys::Single { key: k, ctrl, shift }
                            if k == *key && modifiers_match(*modifiers, ctrl, shift))
                });
                match hit {
                    Some(s) => {
                        actions.push(s.action);
                        false
                    }
                    None => true,
                }
            });
        });
        for a in actions {
            self.run_action(a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_two_shortcuts_share_a_key() {
        let mut seen: Vec<Keys> = Vec::new();
        for s in SHORTCUTS {
            assert!(!seen.contains(&s.keys), "{} is used twice", s.keys.text());
            seen.push(s.keys);
        }
        for (key, what) in RESERVED {
            assert!(
                !seen.contains(&Keys::key(key)),
                "{} is kept for {what}",
                key_name(key)
            );
        }
    }

    #[test]
    fn the_cad_defaults_are_the_keys_asked_for() {
        let expect = [
            (Action::SketchTool(Tool::Line), "L"),
            (Action::SketchTool(Tool::Rectangle), "R"),
            (Action::SketchTool(Tool::Circle), "C"),
            (Action::SketchTool(Tool::Arc), "A"),
            (Action::Dimension, "D"),
            (Action::SketchTool(Tool::Trim), "T"),
            (Action::SketchTool(Tool::Offset), "O"),
            (Action::Construction, "X"),
            (Action::Extrude, "E"),
            (Action::PressPull, "Q"),
            (Action::Delete, "Del"),
            (Action::Cancel, "Esc"),
            (Action::Undo, "Ctrl+Z"),
            (Action::Redo, "Ctrl+Y"),
            (Action::Fit, "F"),
            (Action::Home, "H"),
            (Action::Search, "Ctrl+Space, F"),
            (Action::Assistant, "Ctrl+Shift+A"),
        ];
        for (action, keys) in expect {
            assert_eq!(keys_for(action).as_deref(), Some(keys), "{action:?}");
        }
    }

    #[test]
    fn a_tooltip_gets_the_shortcut_after_its_title() {
        assert_eq!(
            tip("Extrude\nPull profiles out", Action::Extrude),
            "Extrude (E)\nPull profiles out"
        );
        assert_eq!(tip("Revolve", Action::Revolve), "Revolve");
    }

    #[test]
    fn ctrl_o_is_not_read_as_the_offset_key() {
        let ctrl = Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        };
        assert!(!modifiers_match(ctrl, false, false));
        assert!(modifiers_match(Modifiers::NONE, false, false));
        assert!(!modifiers_match(Modifiers::SHIFT, false, false));
    }
}
