---
name: chamfera-packaging
description: "Use when building or changing Chamfera's packages and releases: the Linux AppImage, release tags, and the CI jobs that build them."
---

# Packages and releases

## The AppImage
- `packaging/linux/build-appimage.sh` builds `target/appimage/Chamfera-<version>-x86_64.AppImage`: the desktop app and `chamfera-cli` in one file, with the icon and desktop entry from `assets/icons/` and `packaging/linux/`.
- `Chamfera.AppImage` opens the app; `Chamfera.AppImage cli <args>` runs the command-line tool (`packaging/linux/AppRun`).
- appimagetool is downloaded at a pinned version with a checked SHA-256. Change both together.
- Build on Ubuntu 22.04 (CI does), so the AppImage runs on older glibc versions. GPU drivers are never bundled.
- Check a new AppImage by running `cli --version`, `cli exec body.sample`, and `--sample --screenshot` on a desktop.

## Releases
- `main` only receives releases from `dev` (`skills/chamfera-contributing/SKILL.md`).
- Set the version in the root `Cargo.toml` (`[workspace.package] version`), merge `dev` into `main`, tag `vX.Y.Z` on `main` and push the tag. CI (`.github/workflows/ci.yml`) builds the AppImage and publishes a GitHub release with it.
- Every pushed commit's CI run keeps the AppImage as a downloadable artifact (`chamfera-appimage`).
