# Attribution

Chamfera's own code and artwork (including `assets/icons/chamfera.svg`) are MIT OR Apache-2.0.
Third-party Rust crates keep their own licences; `deny.toml` allows only permissive ones.
Help > About Chamfera shows this file.

## Licence texts of the bundled crates
Every package ships THIRD-PARTY-LICENSES.txt next to the app: the copyright and licence
notices of each of the third-party crates built into it (about 300, most of them MIT OR
Apache-2.0), as their licences ask. `cargo xtask notices` writes it from Cargo.lock and the
crates' own licence files. Where a dual-licensed crate lets us choose, we take it under MIT
(or Apache-2.0, ISC or Zlib) and never under a copyleft option (`self_cell` also offers
GPL-2.0, `r-efi` LGPL-2.1).

## Notable components
| Component | What it does in Chamfera | Licence |
|---|---|---|
| egui, eframe, epaint | The user interface | MIT OR Apache-2.0 |
| winit | Windows and input (desktop) | Apache-2.0 |
| wgpu, naga | GPU drawing (Vulkan, Metal, DirectX 12, WebGPU, WebGL 2) | MIT OR Apache-2.0 |
| rfd | Native file dialogs (desktop) | MIT |
| resvg, usvg | Rasterise the interface icons | Apache-2.0 OR MIT |
| tiny-skia | 2D drawing for the icons | BSD-3-Clause |
| png | Screenshots, thumbnails and render images | MIT OR Apache-2.0 |
| miniz_oxide | Deflate for 3MF packages and PNG images | MIT OR Zlib OR Apache-2.0 |
| ureq, ureq-proto | HTTPS client of the Assistant (desktop) | MIT OR Apache-2.0 |
| rustls | TLS for the Assistant (desktop) | Apache-2.0 OR ISC OR MIT |
| rustls-webpki | Certificate checks | ISC |
| ring | Cryptography under rustls; contains code from BoringSSL and once_cell | Apache-2.0 AND ISC (once_cell parts MIT OR Apache-2.0) |
| rustls-native-certs, schannel, security-framework | Read the operating system's root certificates (no list is bundled) | Apache-2.0 OR ISC OR MIT; MIT; MIT OR Apache-2.0 |
| jiff, jiff-tzdb-platform, jiff-tzdb | The local date in drawing title blocks (desktop; the zone database only on Windows) | Unlicense OR MIT |
| serde, serde_json | Design files, commands and settings | MIT OR Apache-2.0 |
| wasm-bindgen, js-sys, web-sys | The browser app | MIT OR Apache-2.0 |

## Interface icons
| Asset | Origin | Licence |
|---|---|---|
| `assets/icons/ui/*.svg` (file, edit, sketch tools, constraints, features, assembly, views, Browser and status icons) | Original artwork drawn for Chamfera on a 24-point grid (style guide: `docs/design/ui.md`); not derived from any other icon set | MIT OR Apache-2.0, like the code |

## Bundled fonts
The user interface uses the default fonts bundled by egui (`epaint_default_fonts`):

| Font | Licence |
|---|---|
| Hack | MIT (Source Foundry); Bitstream Vera License for the Bitstream Vera parts; DejaVu changes public domain |
| Ubuntu Light | Ubuntu Font Licence 1.0 |
| Noto Emoji | SIL Open Font License 1.1 |
| emoji-icon-font, and egui-icons (a subset of it) | MIT (John Slegers) |

Their licence texts are in THIRD-PARTY-LICENSES.txt (from `fonts/*.txt` in `epaint_default_fonts`).

Sketch text (`sketch.text`) draws its letters from a font Chamfera bundles itself, read with
`skrifa` (MIT OR Apache-2.0, Google's fontations, also used by egui):

| Font | File | Licence |
|---|---|---|
| Lato Regular 2.015, unmodified | `assets/fonts/Lato-Regular.ttf` | SIL Open Font License 1.1. Copyright (c) 2011-2015 by tyPoland Lukasz Dziedzic, with Reserved Font Name "Lato". Full text: `assets/fonts/OFL-Lato.txt`, also appended to THIRD-PARTY-LICENSES.txt |

## Build tools
Used at build time only; none of them ship inside the packages:
- [appimagetool](https://github.com/AppImage/appimagetool) assembles the Linux AppImage.
- [NSIS](https://nsis.sourceforge.io) (zlib licence) builds the Windows installer.
- [ImageMagick](https://imagemagick.org) and [librsvg](https://gitlab.gnome.org/GNOME/librsvg) turn the
  icon SVG into PNG, ICO and ICNS images.
- `winresource` (MIT, a Rust build-dependency) embeds the icon and version into the Windows exe.
- [trunk](https://trunkrs.dev), wasm-bindgen and wasm-opt (Binaryen, Apache-2.0) build the browser app.
