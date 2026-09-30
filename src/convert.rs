//! HWP/HWPX 한 파일을 선택한 포맷으로 변환한다. rhwp 를 라이브러리로 직접 호출한다.

use std::fs;
use std::path::{Path, PathBuf};

use rhwp::wasm_api::HwpDocument;

use crate::{docx, fonts, latex, pdfa};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OutputFormat {
    /// PDF/A-2u — 모든 글자에 유니코드 매핑이 있는 보존용 PDF (LLM 입력용 기본값)
    PdfA,
    /// rhwp 가 만든 PDF 그대로 (텍스트 임베드, PDF/A 메타데이터 없음)
    Pdf,
    /// 문서 전체를 하나의 Markdown 파일로 (이미지는 `<이름>_assets/`)
    Markdown,
    /// 문서 전체를 하나의 UTF-8 텍스트 파일로
    Text,
    /// 페이지별 HTML 을 하나의 파일로
    Html,
    /// Word 문서 (문단·글자 서식, 표, 그림, 머리말·꼬리말, 각주, 수식)
    Docx,
}

impl OutputFormat {
    pub const ALL: [OutputFormat; 6] = [
        OutputFormat::PdfA,
        OutputFormat::Pdf,
        OutputFormat::Docx,
        OutputFormat::Markdown,
        OutputFormat::Text,
        OutputFormat::Html,
    ];

    pub fn label(self) -> &'static str {
        match self {
            OutputFormat::PdfA => "PDF/A-2u (텍스트 보존, 권장)",
            OutputFormat::Pdf => "PDF (일반)",
            OutputFormat::Markdown => "Markdown (.md)",
            OutputFormat::Text => "Text (.txt)",
            OutputFormat::Html => "HTML (.html)",
            OutputFormat::Docx => "Word (.docx)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::PdfA | OutputFormat::Pdf => "pdf",
            OutputFormat::Markdown => "md",
            OutputFormat::Text => "txt",
            OutputFormat::Html => "html",
            OutputFormat::Docx => "docx",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "pdfa" | "pdf/a" | "pdf-a" => Some(OutputFormat::PdfA),
            "pdf" => Some(OutputFormat::Pdf),
            "md" | "markdown" => Some(OutputFormat::Markdown),
            "txt" | "text" => Some(OutputFormat::Text),
            "html" | "htm" => Some(OutputFormat::Html),
            "docx" | "word" => Some(OutputFormat::Docx),
            _ => None,
        }
    }
}

/// 변환 결과 요약.
pub struct Converted {
    pub output: PathBuf,
    pub pages: u32,
    /// 변환은 됐지만 알아야 할 것 (예: 텍스트가 빠진 쪽)
    pub warnings: Vec<String>,
}

/// `input` 옆에 같은 이름 + 새 확장자로 저장한다.
pub fn output_path_for(input: &Path, format: OutputFormat) -> PathBuf {
    input.with_extension(format.extension())
}

/// 진행 상황 보고: (0.0~1.0, 지금 하는 일)
pub type Progress<'a> = &'a dyn Fn(f32, String);

pub fn convert_file(input: &Path, format: OutputFormat, progress: Progress) -> Result<Converted, String> {
    progress(0.0, "문서 읽는 중".into());
    let data = fs::read(input).map_err(|e| format!("파일을 읽을 수 없습니다: {e}"))?;
    let mut doc = HwpDocument::from_bytes(&data).map_err(|e| format!("문서 해석 실패: {e}"))?;

    // 외부 연결 그림(같은 폴더의 이미지)을 rhwp CLI 와 같은 규칙으로 적재한다.
    if let Some(parent) = input.parent() {
        let _ = doc.populate_external_images_from_dir(parent);
    }

    progress(0.05, "쪽 나누는 중".into());
    let pages = doc.page_count();
    if pages == 0 {
        return Err("내보낼 페이지가 없습니다".into());
    }
    let output = output_path_for(input, format);
    let mut warnings = Vec::new();
    let title = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    match format {
        OutputFormat::PdfA | OutputFormat::Pdf => {
            let mut options = rhwp::renderer::pdf::PdfExportOptions::default();
            options.font_paths = fonts::pdf_font_paths();
            options.embed_text = true;
            // rhwp 의 render_pages_pdf_native_with_options 와 같은 두 단계(쪽 SVG → PDF)를
            // 직접 돌려 쪽 단위 진행률을 얻는다.
            let mut svgs = Vec::with_capacity(pages as usize);
            for p in 0..pages {
                progress(0.1 + 0.5 * p as f32 / pages as f32, format!("{}/{pages}쪽 그리는 중", p + 1));
                svgs.push(doc.render_page_svg_native(p).map_err(|e| format!("{}쪽 렌더링 실패: {e}", p + 1))?);
            }
            // 쪽마다 불리는 변환 함수를 감싸 진행률을 보고한다. rhwp 는 글꼴 처리에 실패한 쪽을
            // embed_text=false(글자를 윤곽선으로)로 다시 부르는데, 그 쪽은 텍스트가 사라지므로 센다.
            let mut converted = 0u32;
            let mut outlined = 0u32;
            let pdf = rhwp::renderer::pdf::svgs_to_pdf_with_to_chunk(&svgs, &options, |tree, embed| {
                if embed {
                    converted += 1;
                    progress(
                        0.6 + 0.3 * converted as f32 / pages as f32,
                        format!("{converted}/{pages}쪽 PDF로 (글꼴 넣기)"),
                    );
                } else {
                    outlined += 1;
                }
                rhwp::renderer::pdf::svg2pdf_to_chunk(tree, embed)
            })
            .map_err(|e| format!("PDF 변환 실패: {e}"))?;
            if outlined > 0 {
                warnings.push(format!("{outlined}쪽은 글꼴 문제로 글자가 그림으로 들어가 텍스트가 없습니다"));
            }
            drop(svgs);
            let pdf = if format == OutputFormat::PdfA {
                progress(0.9, "PDF/A 변환 중".into());
                pdfa::to_pdfa2u(&pdf, &title).map_err(|e| format!("PDF/A 변환 실패: {e}"))?
            } else {
                pdf
            };
            progress(0.97, "저장 중".into());
            write_atomic(&output, &pdf)?;
        }
        OutputFormat::Text => {
            let mut out = String::new();
            for p in 0..pages {
                progress(0.1 + 0.85 * p as f32 / pages as f32, format!("{}/{pages}쪽 텍스트 추출", p + 1));
                let text = doc
                    .extract_page_text_native(p)
                    .map_err(|e| format!("{}쪽 텍스트 추출 실패: {e}", p + 1))?;
                out.push_str(normalize_pua(&latex::replace_markers(&text, false)).trim_end_matches('\n'));
                out.push_str("\n\n");
            }
            write_atomic(&output, out.as_bytes())?;
        }
        OutputFormat::Docx => {
            let bytes = docx::write_docx(&doc, &title, progress).map_err(|e| format!("DOCX 변환 실패: {e}"))?;
            progress(0.97, "저장 중".into());
            write_atomic(&output, &bytes)?;
        }
        OutputFormat::Markdown => {
            let md = markdown(&doc, input, pages, progress)?;
            write_atomic(&output, md.as_bytes())?;
        }
        OutputFormat::Html => {
            let mut out = String::from("<!DOCTYPE html>\n<html lang=\"ko\">\n<head>\n<meta charset=\"utf-8\">\n<title>");
            out.push_str(&html_escape(&title));
            out.push_str("</title>\n<style>.rhwp-page{margin:0 auto 24px;box-shadow:0 0 4px #999}</style>\n</head>\n<body>\n");
            for p in 0..pages {
                progress(0.1 + 0.85 * p as f32 / pages as f32, format!("{}/{pages}쪽 HTML 변환", p + 1));
                let html = doc
                    .render_page_html_native(p)
                    .map_err(|e| format!("{}쪽 HTML 변환 실패: {e}", p + 1))?;
                out.push_str(&format!("<section class=\"rhwp-page\" data-page=\"{}\">\n", p + 1));
                out.push_str(&html);
                out.push_str("\n</section>\n");
            }
            out.push_str("</body>\n</html>\n");
            write_atomic(&output, out.as_bytes())?;
        }
    }

    progress(1.0, "완료".into());
    Ok(Converted { output, pages, warnings })
}

fn markdown(doc: &HwpDocument, input: &Path, pages: u32, progress: Progress) -> Result<String, String> {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".into());
    let assets_name = format!("{stem}_assets");
    let assets_dir = input.with_file_name(&assets_name);

    let mut out = String::new();
    for p in 0..pages {
        progress(0.1 + 0.85 * p as f32 / pages as f32, format!("{}/{pages}쪽 Markdown 변환", p + 1));
        let (mut md, images) = doc
            .extract_page_markdown_with_images_native(p)
            .map_err(|e| format!("{}쪽 Markdown 변환 실패: {e}", p + 1))?;
        for (idx, (si, pi, ci, bin_id)) in images.iter().enumerate() {
            let token = format!("[[RHWP_IMAGE:{}]]", idx + 1);
            let Some((mime, bytes)) = image_data(doc, *si, *pi, *ci, *bin_id) else {
                md = md.replace(&token, "");
                continue;
            };
            let ext = match mime.as_str() {
                "image/png" => "png",
                "image/jpeg" => "jpg",
                "image/gif" => "gif",
                "image/bmp" => "bmp",
                "image/webp" => "webp",
                _ => "bin",
            };
            let file_name = format!("{stem}_p{:03}_img{:03}.{ext}", p + 1, idx + 1);
            if fs::create_dir_all(&assets_dir).is_err()
                || fs::write(assets_dir.join(&file_name), &bytes).is_err()
            {
                md = md.replace(&token, "");
                continue;
            }
            // 공백이 든 경로도 링크가 깨지지 않도록 <> 로 감싼다.
            md = md.replace(
                &token,
                &format!("![image {}](<{assets_name}/{file_name}>)", idx + 1),
            );
        }
        out.push_str(&format!("<!-- page {} -->\n\n", p + 1));
        out.push_str(normalize_pua(&latex::replace_markers(&md, true)).trim_end());
        out.push_str("\n\n");
    }
    Ok(out)
}

fn image_data(
    doc: &HwpDocument,
    si: Option<usize>,
    pi: Option<usize>,
    ci: Option<usize>,
    bin_id: u16,
) -> Option<(String, Vec<u8>)> {
    if let (Some(si), Some(pi), Some(ci)) = (si, pi, ci) {
        if let (Ok(mime), Ok(data)) = (
            doc.get_control_image_mime_native(si, pi, &[], ci),
            doc.get_control_image_data_native(si, pi, &[], ci),
        ) {
            return Some((mime, data));
        }
    }
    if bin_id == 0 {
        return None;
    }
    let mime = doc.get_bin_data_image_mime_native(bin_id).ok()?;
    let data = doc.get_bin_data_image_data_native(bin_id).ok()?;
    Some((mime, data))
}

/// 한컴 전용 PUA 문자(원문자 번호, 글머리표, 옛한글 등)를 표준 유니코드로 바꾼다 —
/// PDF 렌더링 경로와 같은 표를 쓴다. LLM 이 알아볼 수 없는 사설 영역 문자를 남기지 않기 위해서다.
pub(crate) fn normalize_pua(text: &str) -> String {
    let is_pua = |c: char| matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD);
    if !text.chars().any(is_pua) {
        return text.to_string();
    }
    // 네모 안 숫자(U+F02B0 = 0 … U+F02C4 = 20)는 대응하는 표준 문자가 없어 [n] 으로 적는다.
    let mut boxed = String::with_capacity(text.len());
    for c in text.chars() {
        match c as u32 {
            cp @ 0xF02B0..=0xF02C4 => boxed.push_str(&format!("[{}]", cp - 0xF02B0)),
            _ => boxed.push(c),
        }
    }
    rhwp::renderer::composer::expand_pua_render_text(&boxed)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// 임시 파일에 쓴 뒤 rename 한다 — 변환 도중 실패해도 반쯤 쓴 결과물이 남지 않는다.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension(format!(
        "{}.part",
        path.extension().and_then(|e| e.to_str()).unwrap_or("out")
    ));
    fs::write(&tmp, bytes).map_err(|e| format!("저장 실패 ({}): {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("저장 실패 ({}): {e}", path.display())
    })
}
