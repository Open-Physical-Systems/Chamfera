---
name: chamfera-packaging
description: "Use when building or changing Chamfera's packages and releases: the Linux AppImage, the Windows zip and installer, the macOS app and disk image, the browser app and its GitHub Pages deployment, release tags, and the CI jobs that build them."
---

# Packages and releases

## The AppImage (Linux)
- `packaging/linux/build-appimage.sh` builds `target/appimage/Chamfera-<version>-x86_64.AppImage`: the desktop app and `chamfera-cli` in one file, with the icon and desktop entry from `assets/icons/` and `packaging/linux/`.
- `Chamfera.AppImage` opens the app; `Chamfera.AppImage cli <args>` runs the command-line tool (`packaging/linux/AppRun`).
- appimagetool is downloaded at a pinned version with a checked SHA-256. Change both together.
- Build on Ubuntu 22.04 (CI does), so the AppImage runs on older glibc versions. GPU drivers are never bundled.
- Check a new AppImage by running `cli --version`, `cli exec body.sample`, and `--sample --screenshot` on a desktop.

## Windows
- `packaging/windows/build-windows.ps1` (PowerShell, on Windows with the MSVC toolchain) builds, in `target/windows/`:
  - `Chamfera-<version>-windows-x86_64.zip`: portable; one folder with `chamfera.exe`, `chamfera-cli.exe`, the licences and `ATTRIBUTION.md`.
  - `Chamfera-<version>-windows-x86_64-setup.exe`: a per-user installer (no administrator rights) into `%LOCALAPPDATA%\Programs\Chamfera`, with a Start menu shortcut and an uninstaller. Silent: `setup.exe /S /D=C:\path`. Built only when NSIS is installed (`-RequireInstaller` makes that an error); the script is `packaging/windows/chamfera.nsi`.
- Tools: ImageMagick (`magick`) turns `assets/icons/chamfera.svg` into `chamfera.ico` at build time; NSIS (zlib/libpng licence) builds the installer. Neither is shipped. WiX is not used (MS-RL, a reciprocal licence); Inno Setup (its own permissive licence) would also do, but NSIS uses a standard one.
- `apps/chamfera/build.rs` embeds the icon and version information with `winresource` (MIT) when the target is Windows and `CHAMFERA_WINDOWS_ICON` names the `.ico`; on other targets it does nothing, and without the variable the exe simply has no icon.
- Release builds of `chamfera.exe` use the Windows GUI subsystem (no console window); `chamfera-cli.exe` stays a console app. `chamfera.exe --version` still prints when its output is redirected.
- Not signed yet: Windows SmartScreen warns on first run. Signing needs a code-signing certificate (or a service such as Azure Trusted Signing); it goes in the script after `cargo build` (sign both exes with `signtool`) and again on the setup exe after `makensis`.

## macOS
- `packaging/macos/build-macos.sh` (on macOS) builds `target/macos/Chamfera-<version>-macos-universal.dmg`: `Chamfera.app` with universal (arm64 + x86_64, joined with `lipo`) `chamfera` and `chamfera-cli` in `Contents/MacOS`, `Info.plist` from `packaging/macos/Info.plist` (bundle id `io.openphysicalsystems.chamfera`, version from Cargo, macOS 11 or newer), and `chamfera.icns` made from the SVG with `rsvg-convert` (or `magick`), `sips` and `iconutil`. The image also has the licences and an Applications link.
- The command-line tool: `/Applications/Chamfera.app/Contents/MacOS/chamfera-cli`.
- Not signed or notarised yet: the bundle is ad-hoc signed only, so on first open users right-click Open (or run `xattr -dr com.apple.quarantine /Applications/Chamfera.app`). Signing needs an Apple Developer ID Application certificate (paid Apple Developer Program). It replaces the ad-hoc `codesign` step in the script (`codesign --options runtime --timestamp --sign "Developer ID Application: ..."`, `chamfera-cli` first, then the app), then the dmg is signed, submitted with `xcrun notarytool submit --wait` and stapled with `xcrun stapler staple`. In CI the certificate and the notary credentials would come from repository secrets.

## The browser app (WebAssembly, GitHub Pages)
- `apps/chamfera-web`: the same UI in the browser. Build: `cargo install --locked trunk`, then `trunk build --release` in that folder writes `dist/` (relative links, `Trunk.toml`). trunk fetches the matching wasm-bindgen and wasm-opt itself. `cargo xtask wasm` only checks that it compiles.
- Size: the `web-release` profile (root `Cargo.toml`: `opt-level = "z"`, fat LTO; only trunk uses it, desktop builds are unchanged) plus `wasm-opt -Oz` give about 8.5 MB of wasm, 3.0 MB gzipped (GitHub Pages serves it compressed).
- Rendering: WebGPU where the browser has it, else WebGL 2; `?renderer=webgl` or `?renderer=webgpu` forces one. On WebGL 2 the viewport re-encodes its picture with a small pass (`RawBlit` in `crates/gpu`), since GL has no texture view formats.
- Files: Open and Insert Mesh use the browser's file picker, Save and Export download the file, through the byte commands (`design.open_bytes`, `design.save_bytes`, `design.export_bytes`, `body.import_stl_bytes`; contents as base64). The current design is kept in the browser's local storage (`chamfera.design`) and restored on load; File > New clears it. It is per browser, not a file: Save downloads the real one. The code is `crates/ui/src/web.rs` and the `wasm32` parts of `file_ui.rs`.
- Check it: `node tools/web-smoke.mjs --out shot.png` serves `dist/`, opens it in headless Chrome (Node 22+, no npm packages), fails on console errors and saves a screenshot; `--no-webgpu` or `--renderer webgl` for the WebGL 2 path, `--scale 2` for HiDPI, `--steps tools/web-checks/files.json` drives open, insert, save, export and local storage with real clicks, keys, file-picker answers and downloads. Look at the screenshot.
- `<body>` tells tests what happened: `data-chamfera` (`running` or `failed`), `data-backend`, `data-error`, `data-message` (the status bar), `data-bodies`, and `data-targets` (the File menu and its items by text, with their centres in CSS pixels). Steps click menu items with `{"clickText": "Export STEP…"}`, never by coordinates, so new menu items do not break the checks.
- Deployment: the `pages` job publishes `dist/` from every push to `dev` to https://open-physical-systems.github.io/Chamfera/ (the pattern is `https://<owner>.github.io/<repository>/`). It runs only for pushes to `dev` in this repository, never for pull requests or forks. One-time setting for the owner: repository Settings > Pages > Build and deployment > Source: **GitHub Actions**. Until then the `pages` job fails and nothing else does.

## CI
- `.github/workflows/ci.yml`: after the `ci` job, `web` builds the browser app and runs the headless-Chrome smoke test and file checks (WebGL 2 on SwiftShader; a WebGPU run is informative only), keeping screenshots as `chamfera-web-screenshots`; on pushes to `dev`, `pages` deploys it. `appimage` (Ubuntu 22.04), `windows` (windows-latest) and `macos` (macos-latest) build the packages, run `chamfera-cli --version`, `chamfera-cli exec body.sample` and `chamfera --version` from the packaged files (the Windows installer is also installed, run and uninstalled silently; the dmg is mounted), and keep them as artifacts: `chamfera-appimage`, `chamfera-windows`, `chamfera-macos`. To save Actions minutes, pull requests run only the Linux `ci` job; `web`, `appimage`, `windows` and `macos` run on pushes to `dev` and on tags.

## Releases
- `main` only receives releases from `dev` (`skills/chamfera-contributing/SKILL.md`).
- Set the version in the root `Cargo.toml` (`[workspace.package] version`), merge `dev` into `main`, tag `vX.Y.Z` on `main` and push the tag. CI builds every package and publishes a GitHub release with the AppImage, the Windows zip and installer, and the macOS dmg.
