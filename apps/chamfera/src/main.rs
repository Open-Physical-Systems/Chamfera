//! The Chamfera desktop app.
//!
//! ```text
//! chamfera [part.stl ...] [--sample] [--theme light|dark] [--script steps.json]
//!          [--parameters] [--configurations] [--tool-library] [--preferences PAGE] [--drawing] [--extrude] [--extrude-with JSON]
//!          [--render] [--render-window WxH,SAMPLES]
//!          [--hole] [--box X1,Y1,X2,Y2] [--drawing-drag X1,Y1,X2,Y2] [--search QUERY] [--context-menu X,Y]
//!          [--press-pull X,Y,D] [--ribbon-menu GROUP] [--assistant-demo]
//!          [--assistant-ask REQUEST] [--about] [--size WxH]
//!          [--history] [--recovery-demo] [--tutorial ID]
//!          [--zoom F] [--screenshot shot.png]
//!          [--sketch-input "tool line; click 0,0; hover 30,12; type 25"]
//! ```

// Release builds on Windows open no console window; chamfera-cli stays a console app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::process::ExitCode;

const USAGE: &str = "usage: chamfera [part.stl ...] [--sample] [--theme light|dark] \
[--script steps.json] [--parameters] [--configurations] [--tool-library] [--preferences PAGE] [--drawing] [--extrude] \
[--extrude-with JSON] [--hole] \
[--box X1,Y1,X2,Y2] [--drawing-drag X1,Y1,X2,Y2] [--search QUERY] [--context-menu X,Y] [--press-pull X,Y,D] \
[--ribbon-menu GROUP] [--assistant-demo] [--assistant-ask REQUEST] [--about] [--history] [--recovery-demo] [--tutorial ID] [--render] [--render-window WxH,SAMPLES] [--size WxH] \
[--zoom F] \
[--screenshot shot.png] \
[--sketch-input \"tool line; click X,Y; hover X,Y; shift; type 25; tab\"]";

/// `WIDTHxHEIGHT,SAMPLES`, each 1 to 4096.
fn parse_render(text: &str) -> Option<[u32; 3]> {
    let (size, samples) = text.split_once(',')?;
    let (w, h) = size.split_once('x')?;
    let v = [w, h, samples].map(|t| t.trim().parse::<u32>().ok());
    let [Some(w), Some(h), Some(s)] = v else {
        return None;
    };
    [w, h, s]
        .iter()
        .all(|n| (1..=4096).contains(n))
        .then_some([w, h, s])
}

fn main() -> ExitCode {
    let mut options = chamfera_ui::Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--sample" => options.sample = true,
            "--parameters" => options.parameters = true,
            "--configurations" => options.configurations = true,
            "--tool-library" => options.tool_library = true,
            "--preferences" => match args.next() {
                Some(page) => options.preferences = Some(page),
                None => {
                    return fail(
                        "--preferences takes a page: general, navigation, units, grid or assistant",
                    );
                }
            },
            "--drawing" => options.drawing = true,
            "--render" => options.render = true,
            "--render-window" => match args.next().as_deref().and_then(parse_render) {
                Some(r) => options.render_window = Some(r),
                None => {
                    return fail(
                        "--render-window takes WIDTHxHEIGHT,SAMPLES, such as 640x400,64 (each 1 to 4096)",
                    );
                }
            },
            "--assistant-demo" => options.assistant_demo = true,
            "--tutorial" => match args.next() {
                Some(id) => options.tutorial = Some(id),
                None => {
                    return fail(
                        "--tutorial takes a tour: first-part, parametric-plate or first-drawing",
                    );
                }
            },
            "--assistant-ask" => match args.next() {
                Some(q) => options.assistant_ask = Some(q),
                None => return fail("--assistant-ask takes a request for the Assistant"),
            },
            "--about" => options.about = true,
            "--history" => options.history = true,
            "--recovery-demo" => options.recovery_demo = true,
            "--extrude" => options.extrude = true,
            "--extrude-with" => match args
                .next()
                .map(|j| serde_json::from_str::<serde_json::Value>(&j))
            {
                Some(Ok(v)) if v.is_object() => {
                    options.extrude = true;
                    options.extrude_with = Some(v);
                }
                _ => return fail("--extrude-with takes a JSON object of solid.extrude options"),
            },
            "--hole" => options.hole = true,
            "--box" => match args.next().as_deref().and_then(parse_box) {
                Some(b) => options.select_box = Some(b),
                None => return fail("--box takes X1,Y1,X2,Y2: viewport pixels, finite"),
            },
            "--drawing-drag" => match args.next().as_deref().and_then(parse_box) {
                Some(b) => {
                    options.drawing_drag = Some(b);
                    options.drawing = true;
                }
                None => return fail("--drawing-drag takes X1,Y1,X2,Y2: sheet mm, 0 to 8192"),
            },
            "--search" => match args.next() {
                Some(q) => options.search = Some(q),
                None => return fail(USAGE),
            },
            "--ribbon-menu" => match args.next() {
                Some(g) => options.ribbon_menu = Some(g),
                None => {
                    return fail("--ribbon-menu takes a ribbon group's caption, such as MODIFY");
                }
            },
            "--press-pull" => match args.next().as_deref().and_then(parse_press_pull) {
                Some(p) => options.press_pull = Some(p),
                None => {
                    return fail(
                        "--press-pull takes X,Y,DISTANCE: a viewport point and a distance in mm",
                    );
                }
            },
            "--context-menu" => match args.next().as_deref().and_then(parse_point) {
                Some(p) => options.context_menu = Some(p),
                None => return fail("--context-menu takes a viewport point such as 600,400"),
            },
            "--size" => match args.next().as_deref().and_then(parse_size) {
                Some(size) => options.window_size = Some(size),
                None => return fail(USAGE),
            },
            "--zoom" => match args.next().and_then(|z| z.parse::<f32>().ok()) {
                Some(z) if z.is_finite() && (0.5..=4.0).contains(&z) => options.zoom = Some(z),
                _ => return fail("--zoom takes a number from 0.5 to 4"),
            },
            "--theme" => match args.next().as_deref() {
                Some("light") => options.theme = Some(chamfera_render::ThemeMode::Light),
                Some("dark") => options.theme = Some(chamfera_render::ThemeMode::Dark),
                _ => return fail(USAGE),
            },
            "--script" => match args.next().map(|p| read_script(&p)) {
                Some(Ok(steps)) => options.script = steps,
                Some(Err(e)) => return fail(&e),
                None => return fail(USAGE),
            },
            "--sketch-input" => match args.next() {
                Some(steps) => options.sketch_input = Some(steps),
                None => return fail(USAGE),
            },
            "--screenshot" => match args.next() {
                Some(p) => options.screenshot = Some(p.into()),
                None => return fail(USAGE),
            },
            "--version" => {
                println!("chamfera {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other if other.starts_with('-') => return fail(USAGE),
            file => options.files.push(file.into()),
        }
    }
    match chamfera_ui::run_native(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("chamfera: {msg}");
    ExitCode::FAILURE
}

/// `600,400` → [600, 400]: a point in the viewport, in points from its top-left.
fn parse_point(text: &str) -> Option<[f32; 2]> {
    let (x, y) = text.split_once(',')?;
    let (x, y) = (x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?);
    let ok = |v: f32| v.is_finite() && (0.0..=8192.0).contains(&v);
    (ok(x) && ok(y)).then_some([x, y])
}

/// `600,400,8` → Press Pull at viewport (600, 400), dragged to 8 mm.
fn parse_press_pull(text: &str) -> Option<[f32; 3]> {
    let (point, d) = text.rsplit_once(',')?;
    let [x, y] = parse_point(point)?;
    let d = d.trim().parse::<f32>().ok()?;
    (d.is_finite() && d.abs() <= 10_000.0).then_some([x, y, d])
}

/// `1000x680` → [1000, 680], within sensible window sizes.
fn parse_size(text: &str) -> Option<[f32; 2]> {
    let (w, h) = text.split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?);
    let ok = |v: f32| v.is_finite() && (320.0..=8192.0).contains(&v);
    (ok(w) && ok(h)).then_some([w, h])
}

/// `100,80,600,400` → the corners of a selection box shown as if being dragged.
fn parse_box(text: &str) -> Option<[f32; 4]> {
    let v: Vec<f32> = text
        .split(',')
        .map(|s| s.trim().parse::<f32>().ok())
        .collect::<Option<_>>()?;
    let ok = |v: &f32| v.is_finite() && (0.0..=8192.0).contains(v);
    match v.as_slice() {
        [a, b, c, d] if v.iter().all(ok) => Some([*a, *b, *c, *d]),
        _ => None,
    }
}

/// Reads a `chamfera-cli run` style script: a JSON list of {"command", "params"}.
fn read_script(path: &str) -> Result<Vec<serde_json::Value>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    v.as_array().cloned().ok_or(format!(
        "{path}: a script is a JSON list of {{\"command\", \"params\"}} steps"
    ))
}
