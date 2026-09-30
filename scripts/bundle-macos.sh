#!/usr/bin/env bash
# release 빌드를 dist/HWP2PDF.app 으로 묶는다 (ad-hoc 서명).
#   UNIVERSAL=1 ./scripts/bundle-macos.sh   arm64 + x86_64 유니버설
#                                           (rustup target add aarch64-apple-darwin x86_64-apple-darwin 필요)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
if [ -n "${UNIVERSAL:-}" ]; then
  for t in aarch64-apple-darwin x86_64-apple-darwin; do cargo build --release --target "$t"; done
  BIN=target/hwp2pdf-universal
  lipo -create target/aarch64-apple-darwin/release/hwp2pdf target/x86_64-apple-darwin/release/hwp2pdf -output "$BIN"
else
  cargo build --release
  BIN=target/release/hwp2pdf
fi

APP="dist/HWP2PDF.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/hwp2pdf"
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
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
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
