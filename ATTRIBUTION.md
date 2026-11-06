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
The AppImage is assembled with [appimagetool](https://github.com/AppImage/appimagetool)
(downloaded at build time, not shipped).
