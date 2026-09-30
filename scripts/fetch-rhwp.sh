#!/usr/bin/env bash
# rhwp 소스를 third_party/rhwp 에 고정 커밋으로 받는다.
# 저장소 전체는 5GB 가 넘으므로(samples·pdf·mydocs) 빌드에 필요한 경로만 sparse checkout 한다.
set -euo pipefail

RHWP_REPO="${RHWP_REPO:-https://github.com/edwardkim/rhwp.git}"
RHWP_REV="${RHWP_REV:-680111ec7bea2fe11110de18c3676ba5a1cf7847}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/third_party/rhwp"

if [ ! -d "$DEST/.git" ]; then
  mkdir -p "$ROOT/third_party"
  git clone --filter=blob:none --no-checkout "$RHWP_REPO" "$DEST"
fi

cd "$DEST"
git sparse-checkout init --no-cone
cat > .git/info/sparse-checkout <<'PATTERNS'
/*
!/samples/
!/pdf/
!/pdf-2020/
!/pdf-large/
!/mydocs/
/mydocs/manual/
/mydocs/tech/agent_roadmap/
!/tools/
/tools/rhwp-subsecond/
/tools/batch-convert/
/tools/llm_verifier/
!/rhwp-studio/
!/rhwp-chrome/
!/rhwp-firefox/
!/rhwp-safari/
!/rhwp-vscode/
!/rhwp-ios/
!/rhwp-desk/
!/fuzz/
PATTERNS
git fetch --depth 1 origin "$RHWP_REV"
git checkout -q -f "$RHWP_REV"

# hwp2pdf 패치 적용 (이미 적용돼 있으면 건너뜀)
for patch in "$ROOT"/patches/rhwp-*.patch; do
  [ -e "$patch" ] || continue
  if git apply --reverse --check "$patch" 2>/dev/null; then
    echo "이미 적용됨: $(basename "$patch")"
  else
    git apply "$patch"
    echo "패치 적용: $(basename "$patch")"
  fi
done
echo "rhwp @ $(git rev-parse --short HEAD) → $DEST"
