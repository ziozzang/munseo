//! 드롭된 경로(파일·폴더)를 변환 대상 목록으로 풀고, 한 파일씩 변환·원본 삭제를 수행한다.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::convert::{self, OutputFormat};

pub fn is_hwp(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    // 숨김 파일, 한글이 편집 중에 만드는 잠금/임시 파일(~$, .~) 은 건너뛴다.
    if name.starts_with('.') || name.starts_with('~') {
        return false;
    }
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("hwp" | "hwpx")
    )
}

/// 파일은 그대로, 폴더는 하위 폴더까지 모두 훑어 HWP/HWPX 파일을 모은다.
pub fn collect(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = BTreeSet::new();
    for path in paths {
        if path.is_dir() {
            for entry in WalkDir::new(path)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    // 숨김 폴더(.git 등)는 들어가지 않는다. 드롭한 폴더 자체는 예외.
                    e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.')
                })
                .filter_map(Result::ok)
            {
                if entry.file_type().is_file() && is_hwp(entry.path()) {
                    found.insert(entry.into_path());
                }
            }
        } else if path.is_file() && is_hwp(path) {
            found.insert(path.clone());
        }
    }
    found.into_iter().collect()
}

#[derive(Debug, Clone)]
pub enum Outcome {
    Ok {
        output: PathBuf,
        pages: u32,
        original_deleted: bool,
        warnings: Vec<String>,
    },
    Failed(String),
}

/// 한 파일을 변환하고, 성공했을 때만 원본을 휴지통으로 옮긴다.
pub fn process(input: &Path, format: OutputFormat, delete_original: bool, progress: convert::Progress) -> Outcome {
    // rhwp 가 특이한 문서에서 panic 하더라도 앱 전체가 죽지 않게 격리한다.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| convert::convert_file(input, format, progress)))
        .unwrap_or_else(|p| {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "알 수 없는 오류".into());
            Err(format!("변환 중 내부 오류(panic): {msg}"))
        });

    match result {
        Ok(done) => {
            let original_deleted = delete_original && trash::delete(input).is_ok();
            Outcome::Ok {
                output: done.output,
                pages: done.pages,
                original_deleted,
                warnings: done.warnings,
            }
        }
        Err(e) => Outcome::Failed(e),
    }
}
