#!/bin/sh
# HWP2PDF macOS 설치 스크립트
#   curl -fsSL https://raw.githubusercontent.com/ziozzang/munseo/main/install.sh | sh
#
# curl 로 받은 파일에는 macOS 격리 속성이 붙지 않아서, 서명·공증이 없어도 Gatekeeper 경고 없이 열린다.
# 최신 릴리스를 받아 SHA256SUMS 로 검증한 뒤 /Applications 에 설치한다 (HWP2PDF_INSTALL_DIR 로 바꿀 수 있음).
set -eu

REPO=ziozzang/munseo
DEST=${HWP2PDF_INSTALL_DIR:-/Applications}

[ "$(uname -s)" = Darwin ] || { echo "macOS 전용입니다. Windows 는 install.ps1 을 쓰세요." >&2; exit 1; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

tag=$(curl -fsSL -H "Accept: application/vnd.github+json" "https://api.github.com/repos/$REPO/releases/latest" \
  | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)
[ -n "$tag" ] || { echo "최신 릴리스를 찾지 못했습니다." >&2; exit 1; }
asset="HWP2PDF_${tag#v}_macos_universal.zip"
base="https://github.com/$REPO/releases/download/$tag"

echo "HWP2PDF $tag 받는 중…"
curl -fL --progress-bar -o "$tmp/$asset" "$base/$asset"
curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS"
(cd "$tmp" && grep " \*\{0,1\}$asset\$" SHA256SUMS | shasum -a 256 -c -) || { echo "체크섬이 맞지 않습니다." >&2; exit 1; }

ditto -x -k "$tmp/$asset" "$tmp/x"
SUDO=""
[ -w "$DEST" ] || SUDO=sudo
mkdir -p "$DEST" 2>/dev/null || $SUDO mkdir -p "$DEST"
$SUDO rm -rf "$DEST/HWP2PDF.app"
$SUDO ditto "$tmp/x/HWP2PDF.app" "$DEST/HWP2PDF.app"
$SUDO xattr -dr com.apple.quarantine "$DEST/HWP2PDF.app" 2>/dev/null || true
# Finder "다음으로 열기" 목록에 바로 나오도록 LaunchServices 에 등록한다.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$DEST/HWP2PDF.app" 2>/dev/null || true

echo "✓ $DEST/HWP2PDF.app ($tag) — 이후 업데이트는 앱이 알아서 합니다."
