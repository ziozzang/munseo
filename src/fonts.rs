//! 내장 폰트(Noto Sans KR, OFL). 한글 폰트가 없는 PC 에서도 PDF 에 글자가 빠지지 않게
//! rhwp 폰트 탐색 경로 맨 앞에 넣고, egui UI 의 한글 표시에도 쓴다.

use std::path::PathBuf;
use std::sync::OnceLock;

pub const NOTO_SANS_KR: &[u8] = include_bytes!("../assets/fonts/NotoSansKR-Regular.ttf");

/// 내장 폰트를 풀어 둔 디렉터리. 시스템 폰트가 우선이고, 없는 글꼴의 폴백으로만 쓰인다.
pub fn pdf_font_paths() -> Vec<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("hwp2pdf-fonts-{}", env!("CARGO_PKG_VERSION")));
        let file = dir.join("NotoSansKR-Regular.ttf");
        let ok = file.metadata().map(|m| m.len() == NOTO_SANS_KR.len() as u64).unwrap_or(false)
            || (std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&file, NOTO_SANS_KR).is_ok());
        ok.then_some(dir)
    })
    .iter()
    .cloned()
    .collect()
}
