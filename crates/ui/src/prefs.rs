//! Per-user preferences: theme, UI scale, default units, recent files, navigation and the
//! grid. They live in a small JSON file in the platform's config folder (in the browser,
//! in local storage), never in the design.
//!
//! The file is hostile input: reading never fails. A missing, unreadable or broken file,
//! or any field of the wrong type or out of range, falls back to that field's default.

use crate::ThemeChoice;
use crate::navigation::{MAX_SENSITIVITY, MIN_SENSITIVITY, Navigation, Preset};
use chamfera_doc::LengthUnit;
use serde_json::{Map, Value, json};

/// Largest preferences file read (bytes).
pub const MAX_PREFS_BYTES: u64 = 256 * 1024;
/// Most recent files remembered.
pub const MAX_RECENT: usize = 30;
/// The UI scale range.
pub const MIN_UI_SCALE: f32 = 0.5;
pub const MAX_UI_SCALE: f32 = 3.0;
/// The fixed grid spacing range (mm).
pub const MIN_GRID: f64 = 0.001;
pub const MAX_GRID: f64 = 100_000.0;
/// Longest path kept in the recent list.
const MAX_PATH_LEN: usize = 4096;
/// The browser's local storage key.
#[cfg(target_arch = "wasm32")]
const STORE_KEY: &str = "chamfera.preferences";

#[derive(Debug, Clone, PartialEq)]
pub struct Preferences {
    pub theme: ThemeChoice,
    /// Interface zoom (1 = 100 %).
    pub ui_scale: f32,
    /// The length unit new designs start with.
    pub default_length: LengthUnit,
    /// How many recent files File > Open Recent lists (0 hides it).
    pub recent_count: usize,
    /// Recently opened or saved designs, most recent first.
    pub recent: Vec<String>,
    pub navigation: Navigation,
    /// A fixed grid spacing in mm, or `None` for the automatic grid.
    pub grid_spacing: Option<f64>,
    /// Sketch clicks on empty space snap to the grid.
    pub snap_to_grid: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            ui_scale: 1.0,
            default_length: LengthUnit::Mm,
            recent_count: 8,
            recent: Vec::new(),
            navigation: Navigation::default(),
            grid_spacing: None,
            snap_to_grid: false,
        }
    }
}

fn theme_key(t: ThemeChoice) -> &'static str {
    match t {
        ThemeChoice::System => "system",
        ThemeChoice::Light => "light",
        ThemeChoice::Dark => "dark",
    }
}

fn theme_parse(s: &str) -> Option<ThemeChoice> {
    match s {
        "system" => Some(ThemeChoice::System),
        "light" => Some(ThemeChoice::Light),
        "dark" => Some(ThemeChoice::Dark),
        _ => None,
    }
}

/// A finite number in a range, else `None`.
fn number(m: &Map<String, Value>, key: &str, lo: f64, hi: f64) -> Option<f64> {
    m.get(key)?
        .as_f64()
        .filter(|v| v.is_finite() && (lo..=hi).contains(v))
}

fn boolean(m: &Map<String, Value>, key: &str) -> Option<bool> {
    m.get(key)?.as_bool()
}

fn text<'a>(m: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    m.get(key)?.as_str()
}

impl Preferences {
    /// The preferences as the file stores them.
    pub fn to_json(&self) -> String {
        let n = &self.navigation;
        let v = json!({
            "format": "chamfera-preferences",
            "version": 1,
            "theme": theme_key(self.theme),
            "ui_scale": self.ui_scale,
            "default_length": self.default_length.symbol(),
            "recent_count": self.recent_count,
            "recent": self.recent,
            "navigation": {
                "preset": n.preset.key(),
                "invert_zoom": n.invert_zoom,
                "zoom_to_cursor": n.zoom_to_cursor,
                "orbit_about_cursor": n.orbit_about_cursor,
                "sensitivity": n.sensitivity,
            },
            "grid_spacing": self.grid_spacing,
            "snap_to_grid": self.snap_to_grid,
        });
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }

    /// Reads preferences, keeping the default for anything missing or invalid.
    pub fn from_json(text_in: &str) -> Preferences {
        let mut p = Preferences::default();
        let Ok(Value::Object(m)) = serde_json::from_str::<Value>(text_in) else {
            return p;
        };
        if let Some(t) = text(&m, "theme").and_then(theme_parse) {
            p.theme = t;
        }
        if let Some(s) = number(
            &m,
            "ui_scale",
            f64::from(MIN_UI_SCALE),
            f64::from(MAX_UI_SCALE),
        ) {
            p.ui_scale = s as f32;
        }
        if let Some(u) = text(&m, "default_length").and_then(LengthUnit::parse) {
            p.default_length = u;
        }
        if let Some(c) = number(&m, "recent_count", 0.0, MAX_RECENT as f64) {
            p.recent_count = c as usize;
        }
        if let Some(Value::Array(list)) = m.get("recent") {
            p.recent = list
                .iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= MAX_PATH_LEN && !s.contains('\0'))
                .take(MAX_RECENT)
                .map(str::to_owned)
                .collect();
        }
        if let Some(Value::Object(n)) = m.get("navigation") {
            let nav = &mut p.navigation;
            if let Some(preset) = text(n, "preset").and_then(Preset::parse) {
                nav.preset = preset;
            }
            if let Some(b) = boolean(n, "invert_zoom") {
                nav.invert_zoom = b;
            }
            if let Some(b) = boolean(n, "zoom_to_cursor") {
                nav.zoom_to_cursor = b;
            }
            if let Some(b) = boolean(n, "orbit_about_cursor") {
                nav.orbit_about_cursor = b;
            }
            if let Some(s) = number(n, "sensitivity", MIN_SENSITIVITY, MAX_SENSITIVITY) {
                nav.sensitivity = s;
            }
        }
        p.grid_spacing = number(&m, "grid_spacing", MIN_GRID, MAX_GRID);
        if let Some(b) = boolean(&m, "snap_to_grid") {
            p.snap_to_grid = b;
        }
        p
    }

    /// Puts a design path at the top of the recent list.
    pub fn add_recent(&mut self, path: &str) {
        if path.is_empty() || path.len() > MAX_PATH_LEN {
            return;
        }
        self.recent.retain(|r| r != path);
        self.recent.insert(0, path.to_owned());
        self.recent.truncate(MAX_RECENT);
    }

    /// The recent files to list, at most `recent_count`.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn recent_shown(&self) -> &[String] {
        let n = self.recent_count.min(self.recent.len());
        self.recent.get(..n).unwrap_or_default()
    }
}

/// The preferences file: `$XDG_CONFIG_HOME/chamfera` or `~/.config/chamfera` on Linux,
/// `~/Library/Application Support/Chamfera` on macOS, `%APPDATA%\Chamfera` on Windows.
#[cfg(not(target_arch = "wasm32"))]
pub fn default_path() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let var = |k: &str| {
        std::env::var_os(k)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let dir = if cfg!(target_os = "windows") {
        var("APPDATA")?.join("Chamfera")
    } else if cfg!(target_os = "macos") {
        var("HOME")?.join("Library/Application Support/Chamfera")
    } else {
        var("XDG_CONFIG_HOME")
            .or_else(|| var("HOME").map(|h| h.join(".config")))?
            .join("chamfera")
    };
    Some(dir.join("preferences.json"))
}

/// Reads preferences from a file; anything wrong gives the defaults.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_from(path: &std::path::Path) -> Preferences {
    match chamfera_io::read_limited(path, MAX_PREFS_BYTES) {
        Ok(bytes) => Preferences::from_json(&String::from_utf8_lossy(&bytes)),
        Err(_) => Preferences::default(),
    }
}

/// Writes preferences to a file (through a temporary file, so a crash never leaves half
/// a file), creating its folder.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_to(prefs: &Preferences, path: &std::path::Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, prefs.to_json()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// The user's preferences.
#[cfg(not(target_arch = "wasm32"))]
pub fn load() -> Preferences {
    default_path().map_or_else(Preferences::default, |p| load_from(&p))
}

/// Saves the user's preferences.
#[cfg(not(target_arch = "wasm32"))]
pub fn save(prefs: &Preferences) -> Result<(), String> {
    let path = default_path().ok_or("no config folder (HOME is not set)")?;
    save_to(prefs, &path)
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

#[cfg(target_arch = "wasm32")]
pub fn load() -> Preferences {
    storage()
        .and_then(|s| s.get_item(STORE_KEY).ok().flatten())
        .filter(|t| t.len() as u64 <= MAX_PREFS_BYTES)
        .map_or_else(Preferences::default, |t| Preferences::from_json(&t))
}

#[cfg(target_arch = "wasm32")]
pub fn save(prefs: &Preferences) -> Result<(), String> {
    storage()
        .ok_or("this browser has no local storage")?
        .set_item(STORE_KEY, &prefs.to_json())
        .map_err(|_| "the browser refused to store the preferences".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed() -> Preferences {
        Preferences {
            theme: ThemeChoice::Dark,
            ui_scale: 1.5,
            default_length: LengthUnit::In,
            recent_count: 4,
            recent: vec!["/a/b.chamfera".into(), "/c.chamfera".into()],
            navigation: Navigation {
                preset: Preset::LeftOrbit,
                invert_zoom: true,
                zoom_to_cursor: false,
                orbit_about_cursor: true,
                sensitivity: 2.0,
            },
            grid_spacing: Some(5.0),
            snap_to_grid: true,
        }
    }

    #[test]
    fn preferences_round_trip_through_their_file() {
        let dir = std::env::temp_dir().join(format!("chamfera-prefs-{}", std::process::id()));
        let path = dir.join("sub").join("preferences.json");
        let p = changed();
        save_to(&p, &path).unwrap();
        assert_eq!(load_from(&path), p);
        // Saving again replaces the file.
        save_to(&Preferences::default(), &path).unwrap();
        assert_eq!(load_from(&path), Preferences::default());
        let _ = std::fs::remove_dir_all(&dir);
        // A missing file gives the defaults.
        assert_eq!(load_from(&dir.join("none.json")), Preferences::default());
    }

    #[test]
    fn hostile_preference_files_never_panic_and_fall_back_per_field() {
        let long = "x".repeat(10_000);
        let nested = (0..10_000).fold(String::from("1"), |s, _| format!("[{s}]"));
        let cases = [
            String::new(),
            "\0\0\0".into(),
            "null".into(),
            "[1,2,3]".into(),
            "{".into(),
            nested,
            format!(r#"{{"theme": "{long}", "ui_scale": 1e308, "recent_count": -5}}"#),
            r#"{"ui_scale": "big", "default_length": 25.4, "recent": "a"}"#.into(),
            r#"{"ui_scale": NaN, "recent": [1, null, "", "\u0000x"]}"#.into(),
            r#"{"navigation": {"preset": "chaos", "sensitivity": 1e9, "invert_zoom": 1}}"#.into(),
            r#"{"navigation": [1], "grid_spacing": -1, "snap_to_grid": "yes"}"#.into(),
            r#"{"grid_spacing": 0, "recent_count": 1e20}"#.into(),
        ];
        for c in &cases {
            let p = Preferences::from_json(c);
            assert!(p.ui_scale.is_finite() && (MIN_UI_SCALE..=MAX_UI_SCALE).contains(&p.ui_scale));
            assert!(p.recent_count <= MAX_RECENT);
            assert!(p.recent.iter().all(|r| !r.is_empty() && !r.contains('\0')));
            let s = p.navigation.sensitivity;
            assert!((MIN_SENSITIVITY..=MAX_SENSITIVITY).contains(&s));
            assert!(
                p.grid_spacing
                    .is_none_or(|g| (MIN_GRID..=MAX_GRID).contains(&g))
            );
        }
        // Good fields survive next to bad ones.
        let p = Preferences::from_json(
            r#"{"theme": "dark", "ui_scale": "big", "navigation": {"preset": "solidworks", "sensitivity": -3}}"#,
        );
        assert_eq!(p.theme, ThemeChoice::Dark);
        assert_eq!(p.ui_scale, 1.0);
        assert_eq!(p.navigation.preset, Preset::SolidWorks);
        assert_eq!(p.navigation.sensitivity, 1.0);
        // A folder, a device and a huge file read as defaults.
        let dir = std::env::temp_dir();
        assert_eq!(load_from(&dir), Preferences::default());
        #[cfg(unix)]
        assert_eq!(
            load_from(std::path::Path::new("/dev/zero")),
            Preferences::default()
        );
        let big = dir.join(format!("chamfera-big-prefs-{}.json", std::process::id()));
        std::fs::write(&big, vec![b' '; (MAX_PREFS_BYTES + 10) as usize]).unwrap();
        assert_eq!(load_from(&big), Preferences::default());
        let _ = std::fs::remove_file(&big);
    }

    #[test]
    fn the_recent_list_puts_the_latest_first_without_repeats() {
        let mut p = Preferences::default();
        p.add_recent("/a");
        p.add_recent("/b");
        p.add_recent("/a");
        assert_eq!(p.recent, vec!["/a".to_owned(), "/b".to_owned()]);
        p.recent_count = 1;
        assert_eq!(p.recent_shown(), &["/a".to_owned()]);
        p.recent_count = 0;
        assert!(p.recent_shown().is_empty());
    }
}
