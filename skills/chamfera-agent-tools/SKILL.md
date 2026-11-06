---
name: chamfera-agent-tools
description: "Use when running Chamfera to see a change working: the desktop app, window screenshots, windowless snapshots of the viewport, and driving the CLI with commands or scripts."
---

# Running Chamfera and looking at it

## The desktop app
- `cargo run -p chamfera -- --sample` opens the window with the sample bracket; give `.stl` paths to insert meshes; `--theme light|dark` forces a theme.
- `cargo run -p chamfera -- --sample --screenshot /path/shot.png` opens the window, waits for it to draw, saves a screenshot of the whole window and quits. Then read the PNG and look at it.
- It needs a display (Wayland or X11). Without one, use the snapshot below.

## Snapshots without a window
`cargo run -p chamfera-cli -- snapshot --sample --out shot.png` renders the viewport with the same renderer the app uses. Options:
- `--stl part.stl` (repeatable) and `--script steps.json`;
- `--view iso|front|back|left|right|top|bottom`;
- `--projection perspective|orthographic`;
- `--theme light|dark`;
- `--size 1280x800`;
- `--pick X,Y`, which prints what that pixel hits and shows it selected.

It uses the GPU if there is one, else a software Vulkan driver (lavapipe, `mesa-vulkan-drivers`).

## Commands and scripts
- `chamfera-cli commands` lists every command; `chamfera-cli exec <id> '<json>'` runs one.
- `chamfera-cli run script.json` runs a list of `{"command", "params"}` steps and prints one reply per line (`examples/parameters.json`).
- Check results with the numbers in the replies (volume, bounds, pick point and normal) against values you worked out, not only with pictures.

## Evidence
For a visible change, attach a screenshot or snapshot you captured and looked at, with before and after when it changes something that existed.
