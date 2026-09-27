#!/usr/bin/env bash
# Builds SquirrelDisk.app from a release binary.
# Usage: scripts/bundle-macos.sh [path/to/squirreldisk] [output dir]
set -euo pipefail
BIN="${1:-target/release/squirreldisk}"
OUT="${2:-target/bundle}"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
APP="$OUT/SquirrelDisk.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/SquirrelDisk"
cp assets/icon/SquirrelDisk.icns "$APP/Contents/Resources/SquirrelDisk.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>SquirrelDisk</string>
  <key>CFBundleDisplayName</key><string>SquirrelDisk</string>
  <key>CFBundleIdentifier</key><string>com.squirreldisk.app</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleExecutable</key><string>SquirrelDisk</string>
  <key>CFBundleIconFile</key><string>SquirrelDisk</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
</dict>
</plist>
PLIST
echo "$APP"
