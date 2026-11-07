# Release checklist

The steps the owner runs to publish a release `vX.Y.Z` (the first one: `v0.1.0`). Only the
owner tags and publishes; agents prepare everything up to step 3. Background:
`skills/chamfera-packaging/SKILL.md` (packages, CI) and `skills/chamfera-contributing/SKILL.md`
(branches).

## 0. Before anything: GitHub Actions must run
The tag builds the Windows and macOS packages on GitHub's machines; nothing else can build
them. As of November 2026 Actions is blocked by a billing problem on the account, so:
- Fix billing first (GitHub > Settings > Billing and plans: a payment method, or a raised
  spending limit for Actions). macOS minutes are billed at ten times, and Windows at twice,
  the Linux rate on private repositories; public repositories run free.
- Check that a push to `dev` runs all of `ci`, `web`, `appimage`, `windows` and `macos`
  green, and download the three artifacts (`chamfera-appimage`, `chamfera-windows`,
  `chamfera-macos`) to try them (step 4) before tagging.
- One-time, for the browser app: Settings > Pages > Source: **GitHub Actions**.

Without Actions, only the Linux and browser builds can be made locally (section 7).

## 1. Prepare on `dev` (a topic branch, merged by pull request)
1. `CHANGELOG.md`: move the `[X.Y.Z] - unreleased` heading to today's date, and add what
   merged since it was written (`gh pr list --state merged -L 100`).
2. Version: `[workspace.package] version = "X.Y.Z"` in the root `Cargo.toml`, then
   `cargo check` so `Cargo.lock` follows. Every crate and app takes its version from there
   (`chamfera --version`, Help > About, the package names, `Info.plist`, the installer).
3. `cargo xtask ci` passes locally (it also checks the licences and writes
   `target/THIRD-PARTY-LICENSES.txt`).
4. Merge into `dev`; wait for the push run on `dev` to pass all jobs.

## 2. Check the packages from that `dev` run
- AppImage (Linux): `chmod +x Chamfera-X.Y.Z-x86_64.AppImage`, then
  `./Chamfera-X.Y.Z-x86_64.AppImage cli --version`, `... cli exec body.sample`,
  `... --sample --screenshot shot.png`, `... --about --screenshot about.png` (licence texts
  shown), and `ANTHROPIC_API_KEY=bogus-key-1234 ./Chamfera-X.Y.Z-x86_64.AppImage
  --assistant-ask hi --screenshot tls.png` (an HTTP 401 in the Assistant panel means TLS and
  the system's root certificates work). Open a file with File > Open (the system dialog).
- Windows: unzip and run `chamfera.exe`; run the installer, start Chamfera from the Start
  menu, check Help > About > Show the licence texts, and uninstall. SmartScreen warns
  (unsigned): More info > Run anyway.
- macOS: open the dmg, drag Chamfera to Applications, right-click > Open (unsigned), check
  About and the Assistant the same way.
- Browser: https://open-physical-systems.github.io/Chamfera/ after the `pages` job.

## 3. Merge `dev` into `main`
Open a pull request from `dev` to `main` titled "Release vX.Y.Z" and merge it (a merge
commit, not a squash, so `main` keeps `dev`'s history).

## 4. Tag and push the tag (owner only)
```sh
git fetch origin
git checkout main && git pull --ff-only
git tag -a vX.Y.Z -m "Chamfera X.Y.Z"
git push origin vX.Y.Z
```
The tag runs every job; `release` then creates the GitHub release with
`Chamfera-X.Y.Z-x86_64.AppImage`, `Chamfera-X.Y.Z-windows-x86_64.zip`,
`Chamfera-X.Y.Z-windows-x86_64-setup.exe` and `Chamfera-X.Y.Z-macos-universal.dmg`.

## 5. Finish the release page
Edit the release on GitHub: replace the generated notes with the version's section of
`CHANGELOG.md`, and add the first-run notes: the packages are not signed yet (Windows
SmartScreen: More info > Run anyway; macOS: right-click > Open, or
`xattr -dr com.apple.quarantine /Applications/Chamfera.app`), and the Assistant needs the
user's own Anthropic API key (Preferences > Assistant).

## 6. After the release
On `dev`, add a new `## [Unreleased]` section at the top of `CHANGELOG.md`.

## 7. Without GitHub Actions: building locally
- AppImage (on Linux; Ubuntu 22.04 or older gives the widest glibc reach):
  `packaging/linux/build-appimage.sh` (needs ImageMagick `convert` and curl) writes
  `target/appimage/Chamfera-X.Y.Z-x86_64.AppImage`.
- Browser app: `cargo install --locked trunk`, then in `apps/chamfera-web`
  `trunk build --release` and `cargo xtask notices --out dist/THIRD-PARTY-LICENSES.txt`;
  check with `node tools/web-smoke.mjs --no-webgpu --steps tools/web-checks/files.json` and
  `--steps tools/web-checks/workspaces.json`.
- Windows: on a Windows machine with Rust (MSVC), ImageMagick and NSIS:
  `pwsh packaging/windows/build-windows.ps1 -RequireInstaller`. It cannot be built from Linux:
  `ring` needs a Windows C toolchain and `winresource` a resource compiler.
- macOS: on a Mac with Xcode command line tools and `brew install librsvg`:
  `packaging/macos/build-macos.sh`.
- Upload them yourself: `gh release create vX.Y.Z --notes-file notes.md <files>` after
  pushing the tag. Never attach a build made from uncommitted changes.
