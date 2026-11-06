# Attribution

Chamfera's own code and artwork (including `assets/icons/chamfera.svg`) are MIT OR Apache-2.0.
Third-party Rust crates keep their own licences; `deny.toml` allows only permissive ones.

## Interface icons
| Asset | Origin | Licence |
|---|---|---|
| `assets/icons/ui/*.svg` (48 icons: file, edit, sketch tools, constraints, features, views, Browser and status icons) | Original artwork drawn for Chamfera on a 24-point grid (style guide: `docs/design/ui.md`); not derived from any other icon set | MIT OR Apache-2.0, like the code |

They are rasterised at run time by [resvg](https://github.com/linebender/resvg) (Apache-2.0 OR
MIT) and tiny-skia (BSD-3-Clause), which ship inside the app.

## Bundled fonts
The user interface uses the default fonts bundled by egui (`epaint_default_fonts`):

| Font | Licence |
|---|---|
| Hack | MIT (Source Foundry); Bitstream Vera License for the Bitstream Vera parts; DejaVu changes public domain |
| Ubuntu Light | Ubuntu Font Licence 1.0 |
| Noto Emoji | SIL Open Font License 1.1 |
| emoji-icon-font, and egui-icons (a subset of it) | MIT (John Slegers) |

The licence texts ship with the crate (`fonts/*.txt` in `epaint_default_fonts`).

## Build tools
Used at build time only; none of them ship inside the packages:
- [appimagetool](https://github.com/AppImage/appimagetool) assembles the Linux AppImage.
- [NSIS](https://nsis.sourceforge.io) (zlib licence) builds the Windows installer.
- [ImageMagick](https://imagemagick.org) and [librsvg](https://gitlab.gnome.org/GNOME/librsvg) turn the
  icon SVG into PNG, ICO and ICNS images.
- `winresource` (MIT, a Rust build-dependency) embeds the icon and version into the Windows exe.
