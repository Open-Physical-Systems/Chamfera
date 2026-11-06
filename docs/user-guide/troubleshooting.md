# Troubleshooting

This page collects the problems you are most likely to meet, what causes them and what to do. If yours is not here, see "Report a bug" at the end.

## The viewport is blank, or says "No GPU device"

Chamfera draws the viewport with Vulkan, Metal or Direct3D through wgpu. When no graphics device can be opened, the status bar says **"No GPU device: the viewport cannot draw."** and the viewport stays empty, while menus and panels still work.

- **Linux:** install your GPU's Vulkan driver (Mesa's `mesa-vulkan-drivers` for Intel and AMD, the vendor driver for NVIDIA). On a machine with no GPU at all (a virtual machine, a remote desktop), the same package's software driver, **lavapipe**, is enough: it is slower but draws everything. Check with `vulkaninfo --summary` (package `vulkan-tools`).
- **Windows and macOS:** update the graphics driver or the system.
- The windowless renderer has the same needs: `chamfera-cli snapshot` fails with an error saying so when there is no adapter.

## Linux: Wayland or X11

Chamfera runs on both Wayland and X11 and picks the one your desktop session uses. It needs a display: started from a plain terminal over SSH, with no `WAYLAND_DISPLAY` or `DISPLAY`, the window cannot open. Use `chamfera-cli` (commands, scripts and `snapshot`) there instead. If the window misbehaves under one display server (for example the window decorations or scaling look wrong), try the other: log in to an X11 (Xorg) session from your login screen. On a Wayland session, starting Chamfera with `WAYLAND_DISPLAY` unset (`env -u WAYLAND_DISPLAY chamfera`) normally makes it fall back to XWayland, as other winit-based programs do.

For a sharp interface on a high-density screen, raise **Preferences > General > UI scale** (see preferences.md).

## A screenshot times out

`chamfera --screenshot shot.png` waits for the window to draw, then saves and quits. If no frame is drawn within 30 seconds it gives up with "screenshot failed: no frame was drawn within 30 s (is the window hidden or covered?)" and exits with an error. Causes: the window opened on a workspace you are not looking at, was minimised or covered, or the compositor does not draw hidden windows. Keep it visible, or use `chamfera-cli snapshot --script steps.json --out shot.png`, which needs no window. A long start-up script (3D toolpaths, a large render) also counts against the time.

## A feature fails in the timeline

A red tile in the timeline is a feature that could not be built after a change (see timeline.md). Hover it, or right-click and choose **Show Error**: the window gives the error and the broken reference, for example a chamfer whose edge "4:bottom 0" belonged to a hole you suppressed.

1. Undo the change (Ctrl+Z) if it was not what you wanted.
2. Or edit the failing feature: pick an edge or face that exists now, or a smaller size.
3. Or fix the feature it refers to (unsuppress it, roll the marker back and change it).

The features before the failed one are built as usual, so you can keep working while you fix it.

## "Not supported yet" and other kernel refusals

Chamfera's geometry kernel is its own and is still growing (`docs/kernel-ladder.md` in the repository lists what is done). When an operation needs geometry it cannot compute exactly yet, it refuses with "not supported yet: …" and the faces involved, and nothing changes. It never guesses with an approximation. Cases you may meet, and what to try:

- **A fillet or chamfer whose end meets a curved face** ("a blend ends at … where … is not planar"): fillet the edges in a different order, fillet them together in one feature, or put the fillet before the feature that made the curved face.
- **A taper on a spline or ellipse profile** is refused on purpose: the offset of such a curve is not an exact surface. Extrude it straight, or draw the profile with lines and arcs.
- **Booleans with spline sides**: joining, cutting and intersecting spline extrudes with planes, cylinders and other extrudes along the same direction works; a cone, sphere or torus reaching a spline side, or a cylinder across it, is refused naming the faces. Change the order, or the direction of one of them.
- **Lofts** need the same number of holes in every section.
- **A fillet too large** ("it would consume 2:top 3; use a smaller size"): make it smaller than the faces next to it.

If an operation you think should work is refused, report it with the script that shows it (below).

## Rebuilds are slow

Every change rebuilds the timeline from the first feature it affects; caches keep the features before it, so most edits, undos and redos take well under a millisecond to tens of milliseconds. What can still take time:

- **Many curved faces**, especially sweeps along bends and revolves: the time goes into making the display mesh. A part with a swept elbow can take about a third of a second per change.
- **3D toolpaths** in the MANUFACTURE workspace are worked out again after every model change, which can take 0.5 to 5 seconds on detailed parts. Suppress the 3D operations (the eye in the tree) while you change the model, then restore them and click Generate.
- **Large simulations**: lower the Simulate panel's resolution to 128 or 256 while checking, and use 1024 only for the final look.

To see where the time goes in a design, run `chamfera-cli bench design-script.json` from a terminal, or the `debug.timings` command, which splits each item's time into kernel, booleans, naming and tessellation.

## After a crash: recovery files

If Chamfera closed without saving, the next start asks **Recover unsaved work from <time>?** Choose **Recover** to get the work back, **Show Later** to keep it for next time, or **Discard** to delete it (see version-history-and-recovery.md). Recovery files are in `~/.local/share/chamfera/recovery` on Linux (or `$XDG_DATA_HOME/chamfera/recovery`), `~/Library/Application Support/Chamfera/Recovery` on macOS and `%LOCALAPPDATA%\Chamfera\Recovery` on Windows. If the status bar says **Autosave failed**, hover it for the reason (often a full disk or a folder without write permission).

Older versions of a saved design are in the `<name>.chamfera.history` folder beside it; open the History panel (File > Version History…) to restore one.

## Reset the preferences

Quit Chamfera, then delete (or rename) the preferences file:

- Linux: `~/.config/chamfera/preferences.json` (or `$XDG_CONFIG_HOME/chamfera/preferences.json`)
- macOS: `~/Library/Application Support/Chamfera/preferences.json`
- Windows: `%APPDATA%\Chamfera\preferences.json`

The next start uses the defaults. Your API key is in `assistant-key` in the same folder; leave it there to keep it, or delete it to remove the key. A broken preferences file never stops the app: bad values fall back to their defaults.

## Odd mouse behaviour

If orbit, pan or box selection do not do what you expect, check **Preferences > Navigation > Mouse preset**; the box under the presets lists what each drag does. On a touchpad, pick **Left-drag orbit (touchpad)**. The status bar always shows the current gestures.

## Report a bug

The project lives at <https://github.com/Open-Physical-Systems/Chamfera>. Before opening an issue, check the open issues there. A good report has:

1. What you did, step by step, and what you expected.
2. What happened instead: the exact message from the status bar, the Feature Error window or the terminal.
3. Your Chamfera version (`chamfera-cli --version`, or the `app.about` command in the search palette), operating system, and graphics driver.
4. A way to reproduce it: the `.chamfera` file, or better, a small script of commands (`chamfera-cli run steps.json`) that shows the problem. The search palette and the Assistant run the same commands, so most problems can be written as a script.
5. A screenshot if it is visual (`chamfera --script steps.json --screenshot shot.png`).

Do not attach designs you cannot share, and never paste your API key.

Related: getting-started.md, timeline.md, version-history-and-recovery.md, preferences.md, manufacture.md.
