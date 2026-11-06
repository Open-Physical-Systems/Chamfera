# Attribution

Chamfera's own code and artwork (including `assets/icons/chamfera.svg`) are MIT OR Apache-2.0.
Third-party Rust crates keep their own licences; `deny.toml` allows only permissive ones.

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
