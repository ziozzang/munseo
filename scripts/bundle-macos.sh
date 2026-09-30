#!/usr/bin/env bash
# release 빌드를 dist/HWP2PDF.app 으로 묶는다 (ad-hoc 서명).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
cargo build --release

APP="dist/HWP2PDF.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/hwp2pdf "$APP/Contents/MacOS/hwp2pdf"
cp assets/fonts/NotoSansKR-OFL.txt "$APP/Contents/Resources/"
cp assets/icon/hwp2pdf.icns "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>HWP2PDF</string>
  <key>CFBundleDisplayName</key><string>HWP2PDF</string>
  <key>CFBundleIdentifier</key><string>net.jioh.hwp2pdf</string>
  <key>CFBundleExecutable</key><string>hwp2pdf</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || echo "경고: ad-hoc 서명 실패 (앱은 그대로 쓸 수 있음)"
echo "→ $ROOT/$APP"
