#!/usr/bin/env bash
# 이 Mac 에서 macOS(유니버설)·Windows(x86_64) 릴리스 파일을 빌드해 GitHub 릴리스로 올린다. CI 는 쓰지 않는다.
#
#   1) Cargo.toml 의 version 과 RELEASE_NOTES.md 를 고치고 커밋
#   2) ./scripts/release.sh            # out/ 에 빌드만
#      ./scripts/release.sh --publish  # 태그 v<버전> 푸시 + 릴리스 생성·업로드 ($GITHUB_TOKEN 필요)
#
# 준비물: brew install rustup mingw-w64
#         rustup target add aarch64-apple-darwin x86_64-apple-darwin x86_64-pc-windows-gnu
set -euo pipefail
cd "$(dirname "$0")/.."

REPO=ziozzang/munseo
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
TAG="v$VERSION"
MAC="Munseo_${VERSION}_macos_universal.zip"
WIN="Munseo_${VERSION}_windows_x86_64.exe"
# 0.1.x(HWP2PDF) 의 자동 업데이트는 예전 이름을 찾으므로 같은 파일을 예전 이름으로도 올린다.
LEGACY_MAC="HWP2PDF_${VERSION}_macos_universal.zip"
LEGACY_WIN="HWP2PDF_${VERSION}_windows_x86_64.exe"

# Homebrew rust 는 호스트용만 있으므로 rustup 툴체인을 앞에 둔다.
if command -v brew >/dev/null && [ -d "$(brew --prefix rustup 2>/dev/null)/bin" ]; then
  export PATH="$(brew --prefix rustup)/bin:$PATH"
fi

[ -d third_party/rhwp ] || ./scripts/fetch-rhwp.sh

echo "▸ macOS universal"
UNIVERSAL=1 ./scripts/bundle-macos.sh
echo "▸ Windows x86_64 (mingw 크로스 빌드)"
CARGO_TARGET_DIR=target-win cargo build --release --target x86_64-pc-windows-gnu

rm -rf out && mkdir out
ditto -c -k --keepParent dist/Munseo.app "out/$MAC"
cp target-win/x86_64-pc-windows-gnu/release/munseo.exe "out/$WIN"
cp "out/$MAC" "out/$LEGACY_MAC"
cp "out/$WIN" "out/$LEGACY_WIN"
(cd out && shasum -a 256 "$MAC" "$WIN" "$LEGACY_MAC" "$LEGACY_WIN" > SHA256SUMS && shasum -a 256 -c SHA256SUMS)

[ "${1:-}" = --publish ] || { echo "✓ out/ 준비됨 (업로드 안 함)"; exit 0; }

: "${GITHUB_TOKEN:?GITHUB_TOKEN 이 필요합니다}"
[ -z "$(git status --porcelain)" ] || { echo "커밋하지 않은 변경이 있습니다." >&2; exit 1; }
git push origin HEAD
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null || git tag -a "$TAG" -m "Munseo $TAG"
git push origin "$TAG"

API="https://api.github.com/repos/$REPO"
AUTH=(-H "Authorization: Bearer $GITHUB_TOKEN" -H "Accept: application/vnd.github+json")
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
body=$(python3 -c 'import json,sys,os; n=open("RELEASE_NOTES.md").read() if os.path.exists("RELEASE_NOTES.md") else ""; print(json.dumps({"tag_name":sys.argv[1],"name":"Munseo "+sys.argv[1],"body":n}))' "$TAG")
curl -fsS "${AUTH[@]}" -d "$body" -o "$tmp" "$API/releases"
upload=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["upload_url"].split("{")[0])' "$tmp")
for f in "out/$MAC" "out/$WIN" "out/$LEGACY_MAC" "out/$LEGACY_WIN" out/SHA256SUMS; do
  curl -fsS "${AUTH[@]}" -H "Content-Type: application/octet-stream" --data-binary @"$f" -o /dev/null "$upload?name=$(basename "$f")"
  echo "uploaded $(basename "$f")"
done
echo "✓ https://github.com/$REPO/releases/tag/$TAG"
