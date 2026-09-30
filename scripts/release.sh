#!/usr/bin/env bash
# Cargo.toml 의 버전으로 v<버전> 태그를 만들어 푸시한다.
# 태그가 올라가면 GitHub Actions(.github/workflows/build.yml)가 macOS·Windows 결과물과
# SHA256SUMS 로 릴리스를 만든다. 릴리스 노트는 RELEASE_NOTES.md 를 쓴다(없으면 자동 생성).
#
#   1) Cargo.toml 의 version 과 RELEASE_NOTES.md 를 고친다
#   2) cargo build (Cargo.lock 갱신) 후 커밋
#   3) ./scripts/release.sh
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
TAG="v$VERSION"

[ -z "$(git status --porcelain)" ] || { echo "커밋하지 않은 변경이 있습니다." >&2; exit 1; }
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && { echo "$TAG 태그가 이미 있습니다." >&2; exit 1; }

git push origin HEAD
git tag -a "$TAG" -m "HWP2PDF $TAG"
git push origin "$TAG"
echo "✓ $TAG 푸시 — Actions 가 끝나면 https://github.com/ziozzang/munseo/releases/tag/$TAG 에 올라갑니다."
