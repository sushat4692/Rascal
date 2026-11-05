#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

APP_NAME="Rascal"
BIN_NAME="rascal"
BUNDLE_ID="im.sush.rascal"
VERSION="$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
BIN="$TARGET_DIR/release/$BIN_NAME"
APP="$TARGET_DIR/release/$APP_NAME.app"
CONTENTS="$APP/Contents"

if [[ -z "$VERSION" ]]; then
  echo "error: could not read version from Cargo.toml" >&2
  exit 1
fi

cargo build --release -p rascal-app

rm -rf "$APP"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources"
cp "$BIN" "$CONTENTS/MacOS/$BIN_NAME"
chmod +x "$CONTENTS/MacOS/$BIN_NAME"
cp "$ROOT/crates/rascal-app/assets/Rascal.icns" "$CONTENTS/Resources/Rascal.icns"

cat > "$CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>ja</string>
    <key>CFBundleDisplayName</key>
    <string>$APP_NAME</string>
    <key>CFBundleExecutable</key>
    <string>$BIN_NAME</string>
    <key>CFBundleIconFile</key>
    <string>Rascal.icns</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
</dict>
</plist>
PLIST

plutil -lint "$CONTENTS/Info.plist" >/dev/null
xattr -cr "$APP" 2>/dev/null || true
codesign --force --sign - --timestamp=none "$APP" >/dev/null
codesign --verify --deep --strict "$APP" >/dev/null
touch "$APP"

echo "Built $APP"
echo "Open with: open \"$APP\""
