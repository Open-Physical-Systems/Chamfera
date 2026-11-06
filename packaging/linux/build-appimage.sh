#!/usr/bin/env bash
# Builds Chamfera-<version>-x86_64.AppImage in target/appimage/.
# Needs: cargo, ImageMagick (`convert`, for the icon) and curl. appimagetool is downloaded
# (pinned version, checksum checked) on first use.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/target/appimage"
APPDIR="$OUT/Chamfera.AppDir"
TOOL_VERSION="1.9.1"
TOOL="$OUT/appimagetool-$TOOL_VERSION-x86_64.AppImage"
TOOL_URL="https://github.com/AppImage/appimagetool/releases/download/$TOOL_VERSION/appimagetool-x86_64.AppImage"
TOOL_SHA256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"
VERSION="$(cargo metadata --no-deps --format-version 1 --manifest-path "$ROOT/Cargo.toml" \
  | python3 -c 'import sys,json;print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="chamfera"))')"

cargo build --release --manifest-path "$ROOT/Cargo.toml" -p chamfera -p chamfera-cli

rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/256x256/apps"
cp "$ROOT/target/release/chamfera" "$ROOT/target/release/chamfera-cli" "$APPDIR/usr/bin/"
cp "$ROOT/packaging/linux/AppRun" "$APPDIR/AppRun"
chmod +x "$APPDIR/AppRun"
cp "$ROOT/packaging/linux/chamfera.desktop" "$APPDIR/chamfera.desktop"
cp "$ROOT/packaging/linux/chamfera.desktop" "$APPDIR/usr/share/applications/"
convert -background none "$ROOT/assets/icons/chamfera.svg" -resize 256x256 "$APPDIR/chamfera.png"
cp "$APPDIR/chamfera.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/chamfera.png"
ln -sf chamfera.png "$APPDIR/.DirIcon"
cp "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$ROOT/ATTRIBUTION.md" "$APPDIR/usr/"
# The licence texts of the bundled crates; Help > About finds them in usr/.
(cd "$ROOT" && cargo xtask notices --out "$APPDIR/usr/THIRD-PARTY-LICENSES.txt")

if [ ! -x "$TOOL" ]; then
  curl -fsSL "$TOOL_URL" -o "$TOOL.part"
  echo "$TOOL_SHA256  $TOOL.part" | sha256sum --check --quiet
  mv "$TOOL.part" "$TOOL"
  chmod +x "$TOOL"
fi
# Runs without FUSE (CI containers, systems without libfuse2).
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$TOOL" --no-appstream "$APPDIR" "$OUT/Chamfera-$VERSION-x86_64.AppImage"
echo "built $OUT/Chamfera-$VERSION-x86_64.AppImage"
