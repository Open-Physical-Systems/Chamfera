#!/usr/bin/env bash
# Builds target/macos/Chamfera-<version>-macos-universal.dmg: Chamfera.app (a universal
# arm64 + x86_64 binary, with chamfera-cli next to it in Contents/MacOS) plus the licences.
# Needs (on macOS): cargo and rustup (both Apple targets are added if missing), Xcode command
# line tools (lipo, iconutil, sips, hdiutil, plutil), and `rsvg-convert` (Homebrew `librsvg`)
# or ImageMagick (`magick`) to rasterise the icon.
# Unsigned: see skills/chamfera-packaging/SKILL.md for signing and notarisation.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT" # rustup reads rust-toolchain.toml from the working directory
OUT="$ROOT/target/macos"
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
VERSION="$(cargo metadata --no-deps --format-version 1 --manifest-path "$ROOT/Cargo.toml" \
  | python3 -c 'import sys,json;print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="chamfera"))')"
NAME="Chamfera-$VERSION-macos-universal"
APP="$OUT/stage/Chamfera.app"

# Same minimum as Info.plist (LSMinimumSystemVersion).
export MACOSX_DEPLOYMENT_TARGET=11.0
rustup target add "${TARGETS[@]}"
for t in "${TARGETS[@]}"; do
  cargo build --release --manifest-path "$ROOT/Cargo.toml" --target "$t" -p chamfera -p chamfera-cli
done

rm -rf "$OUT/stage" "$OUT/chamfera.iconset"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
for bin in chamfera chamfera-cli; do
  lipo -create -output "$APP/Contents/MacOS/$bin" \
    "$ROOT/target/aarch64-apple-darwin/release/$bin" "$ROOT/target/x86_64-apple-darwin/release/$bin"
done
sed "s/@VERSION@/$VERSION/g" "$ROOT/packaging/macos/Info.plist" > "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"

# The icon, generated from the SVG (never committed).
SRC_PNG="$OUT/chamfera-1024.png"
if command -v rsvg-convert > /dev/null; then
  rsvg-convert -w 1024 -h 1024 -o "$SRC_PNG" "$ROOT/assets/icons/chamfera.svg"
elif command -v magick > /dev/null; then
  magick -background none -density 384 "$ROOT/assets/icons/chamfera.svg" -resize 1024x1024 "$SRC_PNG"
else
  echo "need rsvg-convert (brew install librsvg) or magick (brew install imagemagick)" >&2
  exit 1
fi
ICONSET="$OUT/chamfera.iconset"
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z "$s" "$s" "$SRC_PNG" --out "$ICONSET/icon_${s}x${s}.png" > /dev/null
  sips -z $((s * 2)) $((s * 2)) "$SRC_PNG" --out "$ICONSET/icon_${s}x${s}@2x.png" > /dev/null
done
iconutil -c icns -o "$APP/Contents/Resources/chamfera.icns" "$ICONSET"

# The licence texts of the bundled crates; Help > About finds them in Contents/Resources.
cargo xtask notices --out "$OUT/THIRD-PARTY-LICENSES.txt"
for d in "$APP/Contents/Resources" "$OUT/stage"; do
  cp "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$ROOT/ATTRIBUTION.md" "$OUT/THIRD-PARTY-LICENSES.txt" "$d/"
done
ln -s /Applications "$OUT/stage/Applications"

# Ad-hoc signature (no identity): keeps the bundle's code signature consistent, so macOS shows
# "unidentified developer" rather than "damaged". A Developer ID signature and notarisation
# replace this step once the project has an Apple Developer account.
codesign --force --sign - "$APP/Contents/MacOS/chamfera-cli"
codesign --force --sign - "$APP"
codesign --verify --strict --verbose=2 "$APP"

# hdiutil sometimes fails with "Resource busy" on CI machines; retry a few times.
DMG="$OUT/$NAME.dmg"
rm -f "$DMG"
for attempt in 1 2 3 4 5; do
  if hdiutil create -volname "Chamfera $VERSION" -srcfolder "$OUT/stage" -ov -format UDZO "$DMG"; then
    break
  fi
  if [ "$attempt" = 5 ]; then
    echo "hdiutil create failed" >&2
    exit 1
  fi
  sleep 5
done
hdiutil verify "$DMG"
echo "built $DMG"
