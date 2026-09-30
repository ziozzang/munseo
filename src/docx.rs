//! HWP 문서 모델(rhwp IR) → Word(.docx).
//!
//! 조판은 Word 에 맡기고 구조를 옮긴다: 문단 서식, 글자 서식, 표(병합 셀), 그림,
//! 머리말·꼬리말, 각주·미주, 수식(OMML). 글상자의 글은 그 자리에 문단으로 풀어 넣는다.
//! 한글과 같은 쪽 배치는 목표가 아니다.
//!
//! 단위: HWPUNIT = 1/7200 인치. Word twip = 1/1440 인치 → twip = HWPUNIT / 5.
//! 문단 여백·간격은 rhwp IR 에 2배로 저장돼 있다 → twip = 값 / 10.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;

use rhwp::model::control::{AutoNumber, AutoNumberType, Control, FieldType, PageNumberPos};
use rhwp::model::document::{Document, Section};
use rhwp::model::paragraph::{ColumnBreakType, NumberingRestart, Paragraph};
use rhwp::model::page::ColumnDef;
use rhwp::model::shape::ShapeObject;
use rhwp::model::style::{
    Alignment, BorderFill, BorderLine, BorderLineType, CharShape, HeadType, LineSpacingType,
    Numbering, ParaShape, TabDef, UnderlineType,
};
use rhwp::renderer::{format_number, NumberFormat};
use rhwp::model::table::{Table, VerticalAlign};
use rhwp::wasm_api::HwpDocument;

use crate::omml;

pub fn write_docx(hdoc: &HwpDocument, title: &str, progress: crate::convert::Progress) -> Result<Vec<u8>, String> {
    let doc = hdoc.document();
    let mut w = Writer::new(hdoc, doc);
    let total: usize = doc.sections.iter().map(|s| s.paragraphs.len()).sum::<usize>().max(1);
    let mut done = 0usize;

    // 본문
    let mut body = String::new();
    let n = doc.sections.len();
    for (si, section) in doc.sections.iter().enumerate() {
        w.section_headers = SectionParts::default();
        w.outline_numbering_id = section.section_def.outline_numbering_id;
        let mut cols: Option<&ColumnDef> = None;
        w.column_count = 1;
        let mut wrote_any = false;
        for para in &section.paragraphs {
            // 단 수가 바뀌면 그 앞까지를 '이어지는 구역'으로 끊는다 (Word 는 구역 단위로 단을 정한다).
            if let Some(cd) = para.controls.iter().find_map(|c| match c {
                Control::ColumnDef(cd) => Some(cd),
                _ => None,
            }) {
                if wrote_any && cols.map(|c| c.column_count) != Some(cd.column_count) {
                    let sect = w.sect_pr(section, cols, true);
                    let _ = write!(body, "<w:p><w:pPr>{sect}</w:pPr></w:p>");
                }
                cols = Some(cd);
                w.column_count = cd.column_count.max(1);
            }
            if done % 20 == 0 {
                progress(0.1 + 0.8 * done as f32 / total as f32, format!("문단 {done}/{total} 옮기는 중"));
            }
            done += 1;
            w.paragraph(para, &mut body);
            wrote_any = true;
        }
        let sect = w.sect_pr(section, cols, false);
        if si + 1 < n {
            // 마지막이 아닌 구역의 설정은 구역 끝 문단에 단다.
            let _ = write!(body, "<w:p><w:pPr>{sect}</w:pPr></w:p>");
        } else {
            body.push_str(&sect);
        }
    }

    let document_xml = format!(
        "{XML_DECL}<w:document {NS}><w:body>{body}</w:body></w:document>"
    );
    let doc_rels = std::mem::take(&mut w.rels);

    progress(0.9, "각주·머리말·그림 묶는 중".into());
    // 각주·미주 (본문을 다 쓴 뒤에 만든다 — 그 안의 그림은 각주 파트의 관계로 들어간다)
    let footnotes_xml = w.notes_part(false);
    let footnote_rels = std::mem::take(&mut w.rels);
    let endnotes_xml = w.notes_part(true);
    let endnote_rels = std::mem::take(&mut w.rels);

    // ── 패키지 조립
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut overrides = String::new();
    let mut rels = String::new();
    let _ = write!(rels, r#"<Relationship Id="rIdStyles" Type="{R_STYLES}" Target="styles.xml"/>"#);
    let _ = write!(rels, r#"<Relationship Id="rIdSettings" Type="{R_SETTINGS}" Target="settings.xml"/>"#);
    let _ = write!(rels, r#"<Relationship Id="rIdFootnotes" Type="{R_FOOTNOTES}" Target="footnotes.xml"/>"#);
    let _ = write!(rels, r#"<Relationship Id="rIdEndnotes" Type="{R_ENDNOTES}" Target="endnotes.xml"/>"#);
    for r in &doc_rels {
        rels.push_str(&rel_xml(r));
    }
    for (name, xml, part_rels) in std::mem::take(&mut w.hf_parts) {
        let ct = if name.starts_with("header") { CT_HEADER } else { CT_FOOTER };
        let _ = write!(overrides, r#"<Override PartName="/word/{name}" ContentType="{ct}"/>"#);
        if !part_rels.is_empty() {
            files.push((format!("word/_rels/{name}.rels"), rels_xml(&part_rels).into_bytes()));
        }
        files.push((format!("word/{name}"), xml.into_bytes()));
    }
    if !footnote_rels.is_empty() {
        files.push(("word/_rels/footnotes.xml.rels".into(), rels_xml(&footnote_rels).into_bytes()));
    }
    if !endnote_rels.is_empty() {
        files.push(("word/_rels/endnotes.xml.rels".into(), rels_xml(&endnote_rels).into_bytes()));
    }
    for (name, bytes) in std::mem::take(&mut w.media) {
        files.push((format!("word/media/{name}"), bytes));
    }

    let content_types = format!(
        r#"{XML_DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/><Default Extension="jpg" ContentType="image/jpeg"/><Default Extension="gif" ContentType="image/gif"/><Default Extension="bmp" ContentType="image/bmp"/><Default Extension="tiff" ContentType="image/tiff"/><Default Extension="emf" ContentType="image/x-emf"/><Default Extension="wmf" ContentType="image/x-wmf"/><Default Extension="svg" ContentType="image/svg+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/><Override PartName="/word/footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/><Override PartName="/word/endnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>{overrides}</Types>"#
    );
    let root_rels = format!(
        r#"{XML_DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#
    );
    let now = time::OffsetDateTime::now_utc();
    let stamp = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    let core = format!(
        r#"{XML_DECL}<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>{}</dc:title><dc:creator>Munseo</dc:creator><dcterms:created xsi:type="dcterms:W3CDTF">{stamp}</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF">{stamp}</dcterms:modified></cp:coreProperties>"#,
        esc(title)
    );
    let settings = format!(
        r#"{XML_DECL}<w:settings {NS}>{}<w:defaultTabStop w:val="800"/><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>"#,
        if w.uses_even_headers { "<w:evenAndOddHeaders/>" } else { "" }
    );

    files.push(("[Content_Types].xml".into(), content_types.into_bytes()));
    files.push(("_rels/.rels".into(), root_rels.into_bytes()));
    files.push(("docProps/core.xml".into(), core.into_bytes()));
    files.push(("word/document.xml".into(), document_xml.into_bytes()));
    files.push(("word/_rels/document.xml.rels".into(), format!(
        r#"{XML_DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#
    ).into_bytes()));
    files.push(("word/styles.xml".into(), styles_xml(doc).into_bytes()));
    files.push(("word/settings.xml".into(), settings.into_bytes()));
    files.push(("word/footnotes.xml".into(), footnotes_xml.into_bytes()));
    files.push(("word/endnotes.xml".into(), endnotes_xml.into_bytes()));

    zip_files(files)
}

const XML_DECL: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture""#;
const R_STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const R_SETTINGS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings";
const R_FOOTNOTES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes";
const R_ENDNOTES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes";
const R_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const R_HYPERLINK: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
const R_HEADER: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header";
const R_FOOTER: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer";
const CT_HEADER: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
const CT_FOOTER: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml";

struct Rel {
    id: String,
    kind: &'static str,
    target: String,
    external: bool,
}

#[derive(Default)]
struct SectionParts {
    /// (종류 default|even, 관계 id)
    headers: Vec<(&'static str, String)>,
    footers: Vec<(&'static str, String)>,
}

struct Writer<'a> {
    hdoc: &'a HwpDocument,
    doc: &'a Document,
    /// 지금 쓰고 있는 파트(본문/머리말/각주)의 관계 목록
    rels: Vec<Rel>,
    rel_seq: usize,
    media: Vec<(String, Vec<u8>)>,
    /// bin_data_id → media 파일 이름 (파트마다 관계는 따로 만든다)
    media_names: HashMap<u16, Option<String>>,
    footnotes: Vec<(u32, &'a [Paragraph])>,
    endnotes: Vec<(u32, &'a [Paragraph])>,
    hf_parts: Vec<(String, String, Vec<Rel>)>,
    section_headers: SectionParts,
    uses_even_headers: bool,
    drawing_seq: u32,
    /// 본문 폭(twip) — 그림·표가 넘치지 않게 줄인다
    body_width_twip: i64,
    /// 지금 구역의 단 수
    column_count: u16,
    /// 표 안에 들어가 있는 깊이
    table_depth: usize,
    /// 들어 있는 표 칸의 글 폭(twip) 스택
    cell_widths: Vec<i64>,
    /// 문단 번호 카운터
    numbering: NumberingState,
    /// 지금 구역의 개요 번호 정의 (개요 문단의 번호 정의가 0 일 때 쓴다)
    outline_numbering_id: u16,
    /// 구역 끝에서 머리말/꼬리말에 넣을 쪽 번호
    pending_page_number: Option<&'a PageNumberPos>,
}

impl<'a> Writer<'a> {
    fn new(hdoc: &'a HwpDocument, doc: &'a Document) -> Self {
        let body_width_twip = doc
            .sections
            .first()
            .map(|s| {
                let p = &s.section_def.page_def;
                let w = if p.landscape { p.height } else { p.width } as i64;
                (w - p.margin_left as i64 - p.margin_right as i64 - p.margin_gutter as i64) / 5
            })
            .filter(|w| *w > 0)
            .unwrap_or(9000);
        Self {
            hdoc,
            doc,
            rels: Vec::new(),
            rel_seq: 0,
            media: Vec::new(),
            media_names: HashMap::new(),
            footnotes: Vec::new(),
            endnotes: Vec::new(),
            hf_parts: Vec::new(),
            section_headers: SectionParts::default(),
            uses_even_headers: false,
            drawing_seq: 0,
            body_width_twip,
            column_count: 1,
            table_depth: 0,
            cell_widths: Vec::new(),
            numbering: NumberingState::default(),
            outline_numbering_id: 0,
            pending_page_number: None,
        }
    }

    /// 지금 문단이 쓸 수 있는 폭(twip): 표 칸 안이면 칸 폭, 아니면 단 폭
    fn avail_width(&self) -> i64 {
        match self.cell_widths.last() {
            Some(w) => *w,
            None => self.body_width_twip / self.column_count.max(1) as i64,
        }
    }

    fn next_rel_id(&mut self) -> String {
        self.rel_seq += 1;
        format!("rId{}", self.rel_seq)
    }

    // ───────────────────────────── 문단

    /// 한 HWP 문단을 쓴다. 표·글상자 같은 블록 개체가 끼어 있으면 문단을 나눠서 쓴다.
    fn paragraph(&mut self, para: &'a Paragraph, out: &mut String) {
        let chars: Vec<char> = para.text.chars().collect();
        // 목차처럼 점(·)을 글자로 늘어놓아 채운 줄은 Word 에서 넘쳐 줄이 바뀐다 → 점선 채움 오른쪽 탭으로.
        let leader = leader_run(&chars);
        let ppr = self.para_pr(para, leader.is_some());
        let positions = para.control_text_positions();

        // 위치별 컨트롤 목록
        let mut at: Vec<Vec<usize>> = vec![Vec::new(); chars.len() + 1];
        for (ci, _) in para.controls.iter().enumerate() {
            let p = positions.get(ci).copied().unwrap_or(chars.len()).min(chars.len());
            at[p].push(ci);
        }

        let mut runs = String::new();
        if matches!(para.column_type, ColumnBreakType::Column) {
            runs.push_str(r#"<w:r><w:br w:type="column"/></w:r>"#);
        }
        // 문단 머리(개요·번호·글머리표) — 한글이 조판 때 붙이는 번호라 본문 글에는 없다.
        if let Some(head) = self.head_text(para) {
            let _ = write!(
                runs,
                "<w:r>{}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
                self.run_pr(Some(char_shape_at(para, 0))),
                esc(&head)
            );
        }
        // 하이퍼링크: 시작 위치 → (관계 id, 끝 위치)
        let mut links: HashMap<usize, (String, usize)> = HashMap::new();
        for fr in &para.field_ranges {
            if let Some(Control::Field(f)) = para.controls.get(fr.control_idx) {
                if f.field_type == FieldType::Hyperlink && fr.end_char_idx > fr.start_char_idx {
                    if let Some(url) = hyperlink_url(&f.command) {
                        let rid = self.next_rel_id();
                        self.rels.push(Rel { id: rid.clone(), kind: R_HYPERLINK, target: url, external: true });
                        links.insert(fr.start_char_idx, (rid, fr.end_char_idx));
                    }
                }
            }
        }
        let mut link_end: Option<usize> = None;
        let mut has_content = false;
        let mut after_blocks = String::new(); // 문단 뒤에 이어 쓸 블록(글상자 글, 캡션)
        let mut text_buf = String::new();
        let mut cur_shape: Option<u32> = None;

        for pos in 0..=chars.len() {
            if link_end == Some(pos) {
                self.flush_text(&mut runs, &mut text_buf, cur_shape);
                runs.push_str("</w:hyperlink>");
                link_end = None;
            }
            for &ci in &at[pos] {
                self.flush_text(&mut runs, &mut text_buf, cur_shape);
                match &para.controls[ci] {
                    Control::Table(t) => {
                        // 표 앞의 글을 한 문단으로 닫고 표를 쓴다.
                        if link_end.take().is_some() {
                            runs.push_str("</w:hyperlink>");
                        }
                        if has_content {
                            let _ = write!(out, "<w:p>{ppr}{runs}</w:p>");
                        }
                        runs.clear();
                        has_content = false;
                        self.caption(t.caption.as_ref(), true, out);
                        self.table(t, out);
                        self.caption(t.caption.as_ref(), false, out);
                    }
                    other => {
                        if self.inline_control(other, cur_shape, &mut runs, &mut after_blocks) {
                            has_content = true;
                        }
                    }
                }
            }
            let Some(&c) = chars.get(pos) else { break };
            let c = match leader {
                Some((start, _)) if pos == start => '\t',
                Some((start, end)) if pos > start && pos < end => continue,
                _ => c,
            };
            if link_end.is_none() {
                if let Some((rid, end)) = links.get(&pos) {
                    self.flush_text(&mut runs, &mut text_buf, cur_shape);
                    let _ = write!(runs, r#"<w:hyperlink r:id="{rid}">"#);
                    link_end = Some(*end);
                }
            }
            let shape = char_shape_at(para, pos);
            if cur_shape != Some(shape) {
                self.flush_text(&mut runs, &mut text_buf, cur_shape);
                cur_shape = Some(shape);
            }
            match c {
                '\t' => {
                    self.flush_text(&mut runs, &mut text_buf, cur_shape);
                    let _ = write!(runs, "<w:r>{}<w:tab/></w:r>", self.run_pr(cur_shape));
                }
                '\n' => {
                    self.flush_text(&mut runs, &mut text_buf, cur_shape);
                    let _ = write!(runs, "<w:r>{}<w:br/></w:r>", self.run_pr(cur_shape));
                }
                '\u{FFFC}' => {}
                c if (c as u32) < 0x20 => {}
                c => text_buf.push(c),
            }
            has_content = true;
        }
        self.flush_text(&mut runs, &mut text_buf, cur_shape);
        if link_end.is_some() {
            runs.push_str("</w:hyperlink>");
        }

        // 표 뒤에 남은 글(없으면 빈 문단) — 한글에서도 표를 잡는 문단 한 줄이 있다.
        let _ = write!(out, "<w:p>{ppr}{runs}</w:p>");
        out.push_str(&after_blocks);
    }

    fn flush_text(&self, runs: &mut String, buf: &mut String, shape: Option<u32>) {
        if buf.is_empty() {
            return;
        }
        let text = crate::convert::normalize_pua(buf);
        let _ = write!(
            runs,
            "<w:r>{}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
            self.run_pr(shape),
            esc(&text)
        );
        buf.clear();
    }

    /// 문단 안에 놓이는 개체. 글이 생겼으면 true.
    fn inline_control(
        &mut self,
        ctrl: &'a Control,
        shape: Option<u32>,
        runs: &mut String,
        after: &mut String,
    ) -> bool {
        match ctrl {
            Control::Equation(eq) if !eq.script.trim().is_empty() => {
                // 한글 수식 글자 크기(HWPUNIT, 1000 = 10pt) → ½pt
                let half = if eq.font_size > 0 { (eq.font_size / 50).clamp(8, 144) } else { 20 };
                runs.push_str(&omml::script_to_omml(eq.script.trim(), half));
                true
            }
            Control::Picture(pic) => {
                let ok = self.picture(pic.image_attr.bin_data_id, pic.common.width, pic.common.height, runs);
                self.caption(pic.caption.as_ref(), false, after);
                ok
            }
            Control::Shape(shape_obj) => self.shape(shape_obj, runs, after),
            Control::Footnote(f) => {
                let id = self.footnotes.len() as u32 + 1;
                self.footnotes.push((id, &f.paragraphs));
                let _ = write!(
                    runs,
                    "<w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr><w:footnoteReference w:id=\"{id}\"/></w:r>"
                );
                true
            }
            Control::Endnote(e) => {
                let id = self.endnotes.len() as u32 + 1;
                self.endnotes.push((id, &e.paragraphs));
                let _ = write!(
                    runs,
                    "<w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr><w:endnoteReference w:id=\"{id}\"/></w:r>"
                );
                true
            }
            Control::Header(h) => {
                let kind = hf_kind(&h.apply_to);
                if !self.section_headers.headers.iter().any(|(k, _)| *k == kind) {
                    let id = self.header_footer_part(true, &h.paragraphs);
                    self.section_headers.headers.push((kind, id));
                }
                false
            }
            Control::Footer(f) => {
                let kind = hf_kind(&f.apply_to);
                if !self.section_headers.footers.iter().any(|(k, _)| *k == kind) {
                    let id = self.header_footer_part(false, &f.paragraphs);
                    self.section_headers.footers.push((kind, id));
                }
                false
            }
            Control::AutoNumber(an) => {
                let _ = write!(runs, "<w:r>{}{}</w:r>", self.run_pr(shape), auto_number_xml(an));
                true
            }
            Control::PageNumberPos(pn) if pn.position != 0 => {
                // 쪽 번호는 구역 끝(sect_pr)에서 머리말/꼬리말에 넣는다 — 머리말·꼬리말 컨트롤이
                // 이 뒤에 나올 수도 있기 때문이다.
                self.pending_page_number = Some(pn);
                false
            }
            _ => false,
        }
    }

    fn shape(&mut self, obj: &'a ShapeObject, runs: &mut String, after: &mut String) -> bool {
        match obj {
            ShapeObject::Picture(pic) => {
                self.picture(pic.image_attr.bin_data_id, pic.common.width, pic.common.height, runs)
            }
            ShapeObject::Group(g) => {
                let mut any = false;
                for child in &g.children {
                    any |= self.shape(child, runs, after);
                }
                any
            }
            other => {
                // 글상자의 글은 문단으로 풀어 문단 뒤에 넣는다.
                if let Some(tb) = other.drawing().and_then(|d| d.text_box.as_ref()) {
                    for p in &tb.paragraphs {
                        self.paragraph(p, after);
                    }
                }
                false
            }
        }
    }

    fn caption(&mut self, cap: Option<&'a rhwp::model::shape::Caption>, before: bool, out: &mut String) {
        use rhwp::model::shape::CaptionDirection;
        let Some(cap) = cap else { return };
        if matches!(cap.direction, CaptionDirection::Top) != before {
            return;
        }
        for p in &cap.paragraphs {
            self.paragraph(p, out);
        }
    }

    // ───────────────────────────── 서식

    fn para_pr(&self, para: &Paragraph, leader_tab: bool) -> String {
        let mut s = String::from("<w:pPr>");
        if matches!(para.column_type, ColumnBreakType::Page) && !para.page_break_synthesized {
            s.push_str("<w:pageBreakBefore/>");
        }
        if let Some(ps) = self.doc.doc_info.para_shapes.get(para.para_shape_id as usize) {
            let size_pt = self
                .doc
                .doc_info
                .char_shapes
                .get(char_shape_at(para, 0) as usize)
                .map(|c| c.base_size as f64 / 100.0)
                .unwrap_or(10.0);
            let tab_def = self.doc.doc_info.tab_defs.get(ps.tab_def_id as usize);
            let mut xml = para_shape_xml(ps, size_pt, tab_def, self.avail_width());
            if leader_tab {
                let pos = (self.avail_width() - (ps.margin_right / 10) as i64).max(0);
                let tab = format!(r#"<w:tab w:val="right" w:leader="middleDot" w:pos="{pos}"/>"#);
                // 이 문단의 원래 탭은 버린다 — 앞쪽 탭에 먼저 걸리면 쪽 번호가 중간에 선다.
                if let (Some(a), Some(b)) = (xml.find("<w:tabs>"), xml.find("</w:tabs>")) {
                    xml.replace_range(a..b + "</w:tabs>".len(), "");
                }
                xml.insert_str(0, &format!("<w:tabs>{tab}</w:tabs>"));
            }
            s.push_str(&xml);
        }
        s.push_str("</w:pPr>");
        s
    }

    fn run_pr(&self, shape: Option<u32>) -> String {
        let Some(cs) = shape.and_then(|id| self.doc.doc_info.char_shapes.get(id as usize)) else {
            return String::new();
        };
        char_shape_xml(cs, self.doc)
    }

    fn sect_pr(&mut self, section: &Section, cols: Option<&ColumnDef>, continuous: bool) -> String {
        let p = &section.section_def.page_def;
        let (w, h) = if p.landscape { (p.height, p.width) } else { (p.width, p.height) };
        let (w, h) = if w == 0 || h == 0 { (59528, 84188) } else { (w, h) };
        self.place_page_number();
        let mut s = String::from("<w:sectPr>");
        let parts = std::mem::take(&mut self.section_headers);
        for (kind, id) in &parts.headers {
            let _ = write!(s, r#"<w:headerReference w:type="{kind}" r:id="{id}"/>"#);
        }
        for (kind, id) in &parts.footers {
            let _ = write!(s, r#"<w:footerReference w:type="{kind}" r:id="{id}"/>"#);
        }
        let _ = write!(
            s,
            r#"<w:type w:val="{}"/><w:pgSz w:w="{}" w:h="{}"{}/><w:pgMar w:top="{}" w:right="{}" w:bottom="{}" w:left="{}" w:header="{}" w:footer="{}" w:gutter="{}"/>"#,
            if continuous { "continuous" } else { "nextPage" },
            w / 5,
            h / 5,
            if p.landscape { r#" w:orient="landscape""# } else { "" },
            (p.margin_top + p.margin_header) / 5,
            p.margin_right / 5,
            (p.margin_bottom + p.margin_footer) / 5,
            p.margin_left / 5,
            p.margin_header / 5,
            p.margin_footer / 5,
            p.margin_gutter / 5,
        );
        if let Some(cd) = cols.filter(|c| c.column_count > 1) {
            let _ = write!(
                s,
                r#"<w:cols w:num="{}" w:space="{}"{}/>"#,
                cd.column_count,
                cd.spacing as i32 / 5,
                if cd.separator_type != 0 { r#" w:sep="1""# } else { "" }
            );
        }
        s.push_str("</w:sectPr>");
        s
    }

    // ───────────────────────────── 표

    fn table(&mut self, t: &'a Table, out: &mut String) {
        let rows = t.row_count as usize;
        let cols = t.col_count as usize;
        if rows == 0 || cols == 0 || t.cells.is_empty() {
            return;
        }
        let mut widths = column_widths(t);
        // 본문 단 폭보다 넓은 표(쪽 전체에 걸친 제목 표 등)는 단 폭에 맞춰 줄인다.
        if self.table_depth == 0 {
            let cols = self.column_count.max(1) as i64;
            let avail = (self.body_width_twip * 5 / cols).max(1000) as u64;
            let sum: u64 = widths.iter().map(|&w| w as u64).sum();
            if sum > avail {
                for w in &mut widths {
                    *w = (*w as u64 * avail / sum) as u32;
                }
            }
        }
        let total: u32 = widths.iter().sum();

        // 칸 점유 지도: (행, 열) → (셀 번호, 시작칸인지)
        let mut grid: Vec<Vec<Option<(usize, bool)>>> = vec![vec![None; cols]; rows];
        for (i, c) in t.cells.iter().enumerate() {
            let (r0, c0) = (c.row as usize, c.col as usize);
            for r in r0..(r0 + c.row_span.max(1) as usize).min(rows) {
                for cc in c0..(c0 + c.col_span.max(1) as usize).min(cols) {
                    grid[r][cc] = Some((i, r == r0 && cc == c0));
                }
            }
        }

        let _ = write!(
            out,
            r#"<w:tbl><w:tblPr><w:tblW w:w="{}" w:type="dxa"/><w:tblLayout w:type="fixed"/><w:tblCellMar><w:left w:w="{}" w:type="dxa"/><w:right w:w="{}" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid>"#,
            total / 5,
            t.padding.left as u32 / 5,
            t.padding.right as u32 / 5
        );
        for wdt in &widths {
            let _ = write!(out, r#"<w:gridCol w:w="{}"/>"#, wdt / 5);
        }
        out.push_str("</w:tblGrid>");

        for r in 0..rows {
            let height = t
                .cells
                .iter()
                .filter(|c| c.row as usize == r && c.row_span <= 1)
                .map(|c| c.height)
                .max()
                .unwrap_or(0);
            out.push_str("<w:tr>");
            if r == 0 && t.repeat_header || height > 0 {
                out.push_str("<w:trPr>");
                if height > 0 {
                    let _ = write!(out, r#"<w:trHeight w:val="{}" w:hRule="atLeast"/>"#, height / 5);
                }
                if t.repeat_header && t.cells.iter().any(|c| c.row as usize == r && c.is_header) {
                    out.push_str("<w:tblHeader/>");
                }
                out.push_str("</w:trPr>");
            }
            let mut c = 0;
            while c < cols {
                match grid[r][c] {
                    Some((i, true)) => {
                        let cell = &t.cells[i];
                        let span = (cell.col_span.max(1) as usize).min(cols - c);
                        let w: u32 = widths[c..c + span].iter().sum();
                        out.push_str("<w:tc><w:tcPr>");
                        let _ = write!(out, r#"<w:tcW w:w="{}" w:type="dxa"/>"#, w / 5);
                        if span > 1 {
                            let _ = write!(out, r#"<w:gridSpan w:val="{span}"/>"#);
                        }
                        if cell.row_span > 1 {
                            out.push_str(r#"<w:vMerge w:val="restart"/>"#);
                        }
                        out.push_str(&self.cell_borders(cell.border_fill_id));
                        out.push_str(match cell.vertical_align {
                            VerticalAlign::Top => r#"<w:vAlign w:val="top"/>"#,
                            VerticalAlign::Center => r#"<w:vAlign w:val="center"/>"#,
                            VerticalAlign::Bottom => r#"<w:vAlign w:val="bottom"/>"#,
                        });
                        out.push_str("</w:tcPr>");
                        let mut content = String::new();
                        self.table_depth += 1;
                        let pad = (cell.padding.left as i64 + cell.padding.right as i64) / 5;
                        self.cell_widths.push((w as i64 / 5 - pad).max(200));
                        for p in &cell.paragraphs {
                            self.paragraph(p, &mut content);
                        }
                        self.cell_widths.pop();
                        self.table_depth -= 1;
                        // 셀은 문단으로 끝나야 한다.
                        if !content.ends_with("</w:p>") {
                            content.push_str("<w:p/>");
                        }
                        out.push_str(&content);
                        out.push_str("</w:tc>");
                        c += span;
                    }
                    Some((i, false)) => {
                        let cell = &t.cells[i];
                        if cell.col as usize == c {
                            // 위 셀과 세로 병합된 자리
                            let span = (cell.col_span.max(1) as usize).min(cols - c);
                            let w: u32 = widths[c..c + span].iter().sum();
                            let _ = write!(out, r#"<w:tc><w:tcPr><w:tcW w:w="{}" w:type="dxa"/>"#, w / 5);
                            if span > 1 {
                                let _ = write!(out, r#"<w:gridSpan w:val="{span}"/>"#);
                            }
                            out.push_str("<w:vMerge/>");
                            out.push_str(&self.cell_borders(cell.border_fill_id));
                            out.push_str("</w:tcPr><w:p/></w:tc>");
                            c += span;
                        } else {
                            c += 1;
                        }
                    }
                    None => {
                        let _ = write!(
                            out,
                            r#"<w:tc><w:tcPr><w:tcW w:w="{}" w:type="dxa"/></w:tcPr><w:p/></w:tc>"#,
                            widths[c] / 5
                        );
                        c += 1;
                    }
                }
            }
            out.push_str("</w:tr>");
        }
        out.push_str("</w:tbl>");
    }

    fn cell_borders(&self, border_fill_id: u16) -> String {
        let Some(bf) = border_fill_id
            .checked_sub(1)
            .and_then(|i| self.doc.doc_info.border_fills.get(i as usize))
        else {
            return String::new();
        };
        border_fill_xml(bf)
    }

    // ───────────────────────────── 그림

    fn picture(&mut self, bin_id: u16, width: u32, height: u32, runs: &mut String) -> bool {
        if bin_id == 0 {
            return false;
        }
        let name = match self.media_names.get(&bin_id) {
            Some(n) => n.clone(),
            None => {
                let n = self.load_media(bin_id);
                self.media_names.insert(bin_id, n.clone());
                n
            }
        };
        let Some(name) = name else { return false };
        let rid = self.next_rel_id();
        self.rels.push(Rel { id: rid.clone(), kind: R_IMAGE, target: format!("media/{name}"), external: false });

        // EMU = HWPUNIT × 127. 본문 폭을 넘으면 비율대로 줄인다.
        let (mut cx, mut cy) = (width.max(1) as i64 * 127, height.max(1) as i64 * 127);
        let max_cx = self.body_width_twip * 635;
        if cx > max_cx {
            cy = cy * max_cx / cx;
            cx = max_cx;
        }
        self.drawing_seq += 1;
        let id = self.drawing_seq;
        let _ = write!(
            runs,
            r#"<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{id}" name="그림 {id}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{id}" name="{name}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
        );
        true
    }

    fn load_media(&mut self, bin_id: u16) -> Option<String> {
        let mut data = self.hdoc.get_bin_data_image_data_native(bin_id).ok()?;
        let mime = self.hdoc.get_bin_data_image_mime_native(bin_id).unwrap_or_default();
        // 크기 머리글 없는 WMF 는 Word 가 제대로 그리지 못한다 → 머리글을 붙인다.
        if data.starts_with(&[0x01, 0x00, 0x09, 0x00]) || data.starts_with(&[0x02, 0x00, 0x09, 0x00]) {
            data = add_wmf_placeable_header(&data)?;
        }
        let ext = match mime.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpeg",
            "image/gif" => "gif",
            "image/bmp" => "bmp",
            "image/tiff" => "tiff",
            "image/x-emf" | "image/emf" => "emf",
            "image/x-wmf" | "image/wmf" => "wmf",
            "image/svg+xml" => "svg",
            _ => sniff_ext(&data)?,
        };
        let name = format!("image{bin_id}.{ext}");
        self.media.push((name.clone(), data));
        Some(name)
    }

    // ───────────────────────────── 머리말·꼬리말·각주

    fn header_footer_part(&mut self, header: bool, paras: &'a [Paragraph]) -> String {
        let saved = std::mem::take(&mut self.rels);
        // 머리말·꼬리말은 다단과 상관없이 쪽 전체 폭이다.
        let saved_cols = std::mem::replace(&mut self.column_count, 1);
        let mut body = String::new();
        for p in paras {
            self.paragraph(p, &mut body);
        }
        self.column_count = saved_cols;
        if body.is_empty() {
            body.push_str("<w:p/>");
        }
        let part_rels = std::mem::replace(&mut self.rels, saved);
        let idx = self.hf_parts.len() + 1;
        let (tag, name) = if header { ("hdr", format!("header{idx}.xml")) } else { ("ftr", format!("footer{idx}.xml")) };
        let xml = format!("{XML_DECL}<w:{tag} {NS}>{body}</w:{tag}>");
        self.hf_parts.push((name.clone(), xml, part_rels));
        let rid = self.next_rel_id();
        self.rels.push(Rel { id: rid.clone(), kind: if header { R_HEADER } else { R_FOOTER }, target: name, external: false });
        rid
    }

    /// 쪽 번호를 PAGE 필드로 머리말/꼬리말에 넣는다. 이미 있으면 그 끝에 덧붙인다.
    fn place_page_number(&mut self) {
        let Some(pn) = self.pending_page_number.take() else { return };
        let top = matches!(pn.position, 1 | 2 | 3 | 7 | 9);
        let jc = match pn.position {
            1 | 4 => "left",
            3 | 6 => "right",
            _ => "center",
        };
        let para = format!(r#"<w:p><w:pPr><w:jc w:val="{jc}"/></w:pPr>{}</w:p>"#, page_number_runs(pn));
        let list = if top { &self.section_headers.headers } else { &self.section_headers.footers };
        if let Some((_, rid)) = list.iter().find(|(k, _)| *k == "default") {
            let target = self.rels.iter().find(|r| &r.id == rid).map(|r| r.target.clone());
            if let Some(part) = target.and_then(|t| self.hf_parts.iter_mut().find(|p| p.0 == t)) {
                let close = if top { "</w:hdr>" } else { "</w:ftr>" };
                if let Some(pos) = part.1.rfind(close) {
                    part.1.insert_str(pos, &para);
                }
            }
            return;
        }
        let id = self.raw_header_footer_part(top, para);
        if top {
            self.section_headers.headers.push(("default", id));
        } else {
            self.section_headers.footers.push(("default", id));
        }
    }

    /// 이미 만든 본문 XML 로 머리말/꼬리말 파트를 만든다.
    fn raw_header_footer_part(&mut self, header: bool, body: String) -> String {
        let idx = self.hf_parts.len() + 1;
        let (tag, name) = if header { ("hdr", format!("header{idx}.xml")) } else { ("ftr", format!("footer{idx}.xml")) };
        let xml = format!("{XML_DECL}<w:{tag} {NS}>{body}</w:{tag}>");
        self.hf_parts.push((name.clone(), xml, Vec::new()));
        let rid = self.next_rel_id();
        self.rels.push(Rel { id: rid, kind: if header { R_HEADER } else { R_FOOTER }, target: name, external: false });
        self.rels.last().unwrap().id.clone()
    }

    // ───────────────────────────── 문단 번호

    /// 개요·번호·글머리표 문단의 머리 글자. rhwp 조판기(apply_paragraph_numbering)와 같은 규칙.
    fn head_text(&mut self, para: &Paragraph) -> Option<String> {
        let ps = self.doc.doc_info.para_shapes.get(para.para_shape_id as usize)?;
        match ps.head_type {
            HeadType::None => None,
            HeadType::Outline | HeadType::Number => {
                let id = if ps.numbering_id == 0 && ps.head_type == HeadType::Outline {
                    self.outline_numbering_id
                } else {
                    ps.numbering_id
                };
                let fallback;
                let numbering = match id.checked_sub(1).and_then(|i| self.doc.doc_info.numberings.get(i as usize)) {
                    Some(n) => n,
                    // 정의 없는 개요는 한글 기본 모양(1. / 1.1. …)
                    None if ps.head_type == HeadType::Outline => {
                        fallback = default_outline_numbering();
                        &fallback
                    }
                    None => return None,
                };
                let level = (ps.para_level as usize).min(6);
                let counters = self.numbering.advance(id, level, para.numbering_restart);
                let format = &numbering.level_formats[level];
                if format.is_empty() {
                    return None;
                }
                let text = expand_numbering_format(format, &counters, numbering, level);
                if text.is_empty() {
                    return None;
                }
                let gap = numbering.heads[level].text_distance > 0;
                Some(if gap { format!("{text} ") } else { text })
            }
            HeadType::Bullet => {
                let bullet = ps.numbering_id.checked_sub(1).and_then(|i| self.doc.doc_info.bullets.get(i as usize))?;
                if bullet.bullet_char == '\u{FFFF}' {
                    return None; // 그림 글머리표
                }
                let ch = rhwp::renderer::layout::map_pua_bullet_char(bullet.bullet_char);
                Some(if bullet.text_distance > 0 { format!("{ch} ") } else { ch.to_string() })
            }
        }
    }

    fn notes_part(&mut self, endnotes: bool) -> String {
        let (root, item, reference) = if endnotes {
            ("endnotes", "endnote", "endnoteRef")
        } else {
            ("footnotes", "footnote", "footnoteRef")
        };
        let notes = if endnotes { std::mem::take(&mut self.endnotes) } else { std::mem::take(&mut self.footnotes) };
        let mut s = format!(
            r#"{XML_DECL}<w:{root} {NS}><w:{item} w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:{item}><w:{item} w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:{item}>"#
        );
        for (id, paras) in notes {
            let mut body = String::new();
            for p in paras {
                self.paragraph(p, &mut body);
            }
            // 첫 문단 앞에 번호 표식을 넣는다.
            let mark = format!(
                r#"<w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:{reference}/></w:r><w:r><w:t xml:space="preserve"> </w:t></w:r>"#
            );
            if let Some(pos) = body.find("</w:pPr>") {
                body.insert_str(pos + "</w:pPr>".len(), &mark);
            } else {
                body = format!("<w:p>{mark}</w:p>{body}");
            }
            let _ = write!(s, r#"<w:{item} w:id="{id}">{body}</w:{item}>"#);
        }
        let _ = write!(s, "</w:{root}>");
        s
    }
}

/// 문단 번호 카운터 — rhwp 조판기의 NumberingState 와 같은 규칙.
/// 같은 번호 정의가 이어지면 계속 세고, 다른 정의로 갔다가 돌아오면 그때 값을 되살린다.
#[derive(Default)]
struct NumberingState {
    current_id: Option<u16>,
    counters: [u32; 7],
    history: HashMap<u16, [u32; 7]>,
}

impl NumberingState {
    fn advance(&mut self, id: u16, level: usize, restart: Option<NumberingRestart>) -> [u32; 7] {
        if self.current_id != Some(id) {
            if let Some(prev) = self.current_id {
                self.history.insert(prev, self.counters);
            }
            if let Some(saved) = self.history.get(&id).copied() {
                self.counters = saved;
            } else {
                let prev = self.counters;
                self.counters = [0; 7];
                self.counters[..level].copy_from_slice(&prev[..level]);
            }
            self.current_id = Some(id);
        }
        if let Some(NumberingRestart::NewStart(start)) = restart {
            self.counters[level] = start.saturating_sub(1);
            for c in &mut self.counters[level + 1..] {
                *c = 0;
            }
        }
        self.counters[level] += 1;
        for c in &mut self.counters[level + 1..] {
            *c = 0;
        }
        self.counters
    }
}

fn default_outline_numbering() -> Numbering {
    let mut n = Numbering::default();
    for f in n.level_formats.iter_mut() {
        *f = "^N".to_string();
    }
    n.start_number = 1;
    n.level_start_numbers = [1; 7];
    n
}

/// 번호 형식 문자열의 `^1`~`^7`(수준 번호), `^n`(1.1.1), `^N`(1.1.1.) 을 치환한다.
fn expand_numbering_format(format: &str, counters: &[u32; 7], numbering: &Numbering, level: usize) -> String {
    let level_text = |idx: usize| -> String {
        let start = numbering.level_start_numbers[idx];
        let n = if counters[idx] > 0 { start.saturating_sub(1) + counters[idx] } else { start };
        // HWP 표 43 번호 모양
        let fmt = match numbering.heads[idx].number_format {
            1 => NumberFormat::CircledDigit,
            2 => NumberFormat::RomanUpper,
            3 => NumberFormat::RomanLower,
            4 => NumberFormat::LatinUpper,
            5 => NumberFormat::LatinLower,
            8 => NumberFormat::HangulGaNaDa,
            12 => NumberFormat::HangulNumber,
            13 => NumberFormat::HanjaNumber,
            _ => NumberFormat::Digit,
        };
        format_number(n.min(u16::MAX as u32) as u16, fmt)
    };
    let mut out = String::new();
    let mut it = format.chars().peekable();
    while let Some(c) = it.next() {
        if c == '^' {
            match it.peek().copied() {
                Some(d @ '1'..='7') => {
                    it.next();
                    out.push_str(&level_text((d as u8 - b'1') as usize));
                    continue;
                }
                Some(code @ ('n' | 'N')) => {
                    it.next();
                    for idx in 0..=level {
                        if idx > 0 {
                            out.push('.');
                        }
                        out.push_str(&level_text(idx));
                    }
                    if code == 'N' {
                        out.push('.');
                    }
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}

/// 한글 하이퍼링크 명령(`http\://host/path;1;0;0;`)에서 외부 URL 만 꺼낸다.
/// 문서 안 책갈피(`?#…`)와 `javascript:` 는 버린다.
fn hyperlink_url(command: &str) -> Option<String> {
    let mut url = String::new();
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    url.push(n);
                }
            }
            ';' => break,
            c => url.push(c),
        }
    }
    let url = url.trim().trim_end_matches(')').to_string();
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:") || lower.starts_with("ftp://") {
        Some(url)
    } else if lower.starts_with("www.") {
        Some(format!("http://{url}"))
    } else {
        None
    }
}

/// 자동 번호(그림·표·수식 번호, 쪽 번호). 쪽 번호는 Word PAGE 필드로 넣어 Word 가 매기게 한다.
fn auto_number_xml(an: &AutoNumber) -> String {
    let wrap = |s: String| -> String {
        let mut t = String::new();
        if an.prefix_char != '\0' {
            t.push(an.prefix_char);
        }
        t.push_str(&s);
        if an.suffix_char != '\0' {
            t.push(an.suffix_char);
        }
        t
    };
    match an.number_type {
        AutoNumberType::Page | AutoNumberType::TotalPage => {
            let instr = if matches!(an.number_type, AutoNumberType::Page) { "PAGE" } else { "NUMPAGES" };
            format!(
                r#"<w:t xml:space="preserve">{}</w:t></w:r><w:fldSimple w:instr="{instr}"><w:r><w:t>1</w:t></w:r></w:fldSimple><w:r><w:t xml:space="preserve">{}</w:t>"#,
                if an.prefix_char != '\0' { esc(&an.prefix_char.to_string()) } else { String::new() },
                if an.suffix_char != '\0' { esc(&an.suffix_char.to_string()) } else { String::new() },
            )
        }
        _ => {
            let n = format_number(an.assigned_number, NumberFormat::from_hwp_format(an.format));
            format!(r#"<w:t xml:space="preserve">{}</w:t>"#, esc(&wrap(n)))
        }
    }
}

/// 쪽 번호 위치 컨트롤 → "- PAGE -" 같은 런
fn page_number_runs(pn: &PageNumberPos) -> String {
    let mut before = String::new();
    let mut after = String::new();
    if pn.dash_char != '\0' && pn.dash_char != ' ' {
        before.push(pn.dash_char);
        before.push(' ');
        after.push(' ');
        after.push(pn.dash_char);
    }
    if pn.prefix_char != '\0' {
        before.push(pn.prefix_char);
    }
    if pn.suffix_char != '\0' {
        after.insert(0, pn.suffix_char);
    }
    format!(
        r#"<w:r><w:t xml:space="preserve">{}</w:t></w:r><w:fldSimple w:instr="PAGE"><w:r><w:t>1</w:t></w:r></w:fldSimple><w:r><w:t xml:space="preserve">{}</w:t></w:r>"#,
        esc(&before),
        esc(&after)
    )
}

/// 문단 끝의 "……… 12" 같은 글자 점선 채움 구간 [시작, 끝). 점 6개 이상, 뒤에는 공백과 숫자만.
fn leader_run(chars: &[char]) -> Option<(usize, usize)> {
    let is_dot = |c: char| matches!(c, '·' | '.' | '…' | '‥' | '∙' | '・' | '⋯');
    // 끝에서부터 숫자·공백을 건너뛴다.
    let mut end = chars.len();
    while end > 0 && (chars[end - 1].is_ascii_digit() || chars[end - 1] == ' ' || matches!(chars[end - 1], '-' | '~')) {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && is_dot(chars[start - 1]) {
        start -= 1;
    }
    let dots: usize = chars[start..end].iter().map(|&c| if c == '…' || c == '⋯' { 3 } else { 1 }).sum();
    (dots >= 6 && end < chars.len() && start > 0).then_some((start, end))
}

fn hf_kind(apply: &rhwp::model::header_footer::HeaderFooterApply) -> &'static str {
    use rhwp::model::header_footer::HeaderFooterApply;
    match apply {
        HeaderFooterApply::Even => "even",
        _ => "default",
    }
}

/// 이 글자 위치에 적용되는 글자 모양 번호
fn char_shape_at(para: &Paragraph, char_idx: usize) -> u32 {
    let pos = para.char_offsets.get(char_idx).copied().unwrap_or(char_idx as u32);
    let mut id = para.char_shapes.first().map(|r| r.char_shape_id).unwrap_or(0);
    for r in &para.char_shapes {
        if r.start_pos <= pos {
            id = r.char_shape_id;
        } else {
            break;
        }
    }
    id
}

fn font_name(doc: &Document, lang: usize, id: u16) -> Option<&str> {
    doc.doc_info
        .font_faces
        .get(lang)
        .and_then(|faces| faces.get(id as usize))
        .map(|f| f.name.as_str())
        .filter(|n| !n.is_empty())
}

fn char_shape_xml(cs: &CharShape, doc: &Document) -> String {
    let mut s = String::from("<w:rPr>");
    let hangul = font_name(doc, 0, cs.font_ids[0]);
    let latin = font_name(doc, 1, cs.font_ids[1]).or(hangul);
    if hangul.is_some() || latin.is_some() {
        let (h, l) = (esc(hangul.or(latin).unwrap()), esc(latin.or(hangul).unwrap()));
        let _ = write!(s, r#"<w:rFonts w:ascii="{l}" w:hAnsi="{l}" w:eastAsia="{h}" w:cs="{l}" w:hint="eastAsia"/>"#);
    }
    if cs.bold {
        s.push_str("<w:b/><w:bCs/>");
    }
    if cs.italic {
        s.push_str("<w:i/><w:iCs/>");
    }
    if cs.strikethrough {
        s.push_str("<w:strike/>");
    }
    // 요소 순서는 OOXML 스키마 순서를 지킨다 (b i strike outline shadow emboss imprint color spacing w sz u shd vertAlign).
    if cs.outline_type != 0 {
        s.push_str("<w:outline/>");
    }
    if cs.shadow_type != 0 {
        s.push_str("<w:shadow/>");
    }
    if cs.emboss {
        s.push_str("<w:emboss/>");
    }
    if cs.engrave {
        s.push_str("<w:imprint/>");
    }
    let color = rgb(cs.text_color);
    if color != "000000" {
        let _ = write!(s, r#"<w:color w:val="{color}"/>"#);
    }
    // 자간(%) → twip: 글자 크기 × 자간 / 100
    let size_pt = cs.base_size as f64 / 100.0;
    let spacing = cs.spacings[0] as f64;
    if spacing != 0.0 {
        let _ = write!(s, r#"<w:spacing w:val="{}"/>"#, (size_pt * spacing / 100.0 * 20.0).round() as i32);
    }
    // 장평(%)
    if cs.ratios[0] != 100 && cs.ratios[0] > 0 {
        let _ = write!(s, r#"<w:w w:val="{}"/>"#, cs.ratios[0]);
    }
    if cs.base_size > 0 {
        let half = (cs.base_size / 50).max(2);
        let _ = write!(s, r#"<w:sz w:val="{half}"/><w:szCs w:val="{half}"/>"#);
    }
    if !matches!(cs.underline_type, UnderlineType::None) {
        s.push_str(r#"<w:u w:val="single"/>"#);
    }
    // 음영(0xFFFFFFFF = 없음)
    if cs.shade_color != 0xFFFF_FFFF && cs.shade_color & 0x00FF_FFFF != 0x00FF_FFFF {
        let _ = write!(s, r#"<w:shd w:val="clear" w:color="auto" w:fill="{}"/>"#, rgb(cs.shade_color));
    }
    if cs.superscript {
        s.push_str(r#"<w:vertAlign w:val="superscript"/>"#);
    } else if cs.subscript {
        s.push_str(r#"<w:vertAlign w:val="subscript"/>"#);
    }
    s.push_str("</w:rPr>");
    s
}

fn para_shape_xml(ps: &ParaShape, size_pt: f64, tab_def: Option<&TabDef>, avail_twip: i64) -> String {
    // 요소 순서는 OOXML 스키마 순서(spacing → ind → jc → outlineLvl)를 지킨다 — Word 는 어긋나면 열지 않는다.
    let mut s = String::new();
    // 탭 위치 (문단 여백과 같이 2배로 저장됨). Word 탭은 본문 왼쪽 끝 기준이다.
    if let Some(td) = tab_def.filter(|t| !t.tabs.is_empty() || t.auto_tab_right) {
        // 들어 있는 칸(단) 폭을 넘는 탭은 Word 가 다음 줄로 넘기므로 폭 안으로 당긴다.
        let max_pos = (avail_twip - (ps.margin_right / 10) as i64).max(0);
        s.push_str("<w:tabs>");
        for t in &td.tabs {
            let leader = if t.fill_type != 0 { r#" w:leader="dot""# } else { "" };
            let pos = ((ps.margin_left / 10 + t.position as i32 / 10) as i64).min(max_pos);
            let val = match t.tab_type {
                1 => "right",
                2 => "center",
                3 => "decimal",
                // 오른쪽 끝 근처의 채움 왼쪽 탭(목차의 "……… 쪽번호")은 한글에서 번호가
                // 넘치지 않는다. Word 는 줄을 넘기므로 오른쪽 끝 오른쪽 탭으로 바꾼다.
                _ if t.fill_type != 0 && pos * 10 >= max_pos * 8 => {
                    let _ = write!(s, r#"<w:tab w:val="right"{leader} w:pos="{max_pos}"/>"#);
                    continue;
                }
                _ => "left",
            };
            let _ = write!(s, r#"<w:tab w:val="{val}"{leader} w:pos="{pos}"/>"#);
        }
        if td.auto_tab_right {
            let _ = write!(s, r#"<w:tab w:val="right" w:pos="{max_pos}"/>"#);
        }
        s.push_str("</w:tabs>");
    }
    // 줄 간격: 한글의 % 는 글자 크기 대비 → 최소 줄 높이로 옮긴다(수식·그림이 잘리지 않게).
    let line = match ps.line_spacing_type {
        LineSpacingType::Percent => {
            let pct = if ps.line_spacing > 0 { ps.line_spacing as f64 } else { 160.0 };
            format!(r#" w:line="{}" w:lineRule="atLeast""#, (size_pt * pct / 100.0 * 20.0).round() as i64)
        }
        LineSpacingType::Fixed => format!(r#" w:line="{}" w:lineRule="exact""#, (ps.line_spacing / 10).max(1)),
        LineSpacingType::Minimum => format!(r#" w:line="{}" w:lineRule="atLeast""#, (ps.line_spacing / 10).max(1)),
        LineSpacingType::SpaceOnly => String::new(),
    };
    let _ = write!(
        s,
        r#"<w:spacing w:before="{}" w:after="{}"{line}/>"#,
        (ps.spacing_before / 10).max(0),
        (ps.spacing_after / 10).max(0)
    );
    // 들여쓰기: 양수 = 첫 줄 들여쓰기, 음수 = 내어쓰기(둘째 줄부터 |indent| 만큼 안쪽)
    let left = ps.margin_left / 10;
    let right = ps.margin_right / 10;
    let indent = ps.indent / 10;
    if indent >= 0 {
        let _ = write!(s, r#"<w:ind w:left="{left}" w:right="{right}" w:firstLine="{indent}"/>"#);
    } else {
        let _ = write!(s, r#"<w:ind w:left="{}" w:right="{right}" w:hanging="{}"/>"#, left - indent, -indent);
    }
    s.push_str(match ps.alignment {
        Alignment::Justify => r#"<w:jc w:val="both"/>"#,
        Alignment::Left => r#"<w:jc w:val="left"/>"#,
        Alignment::Right => r#"<w:jc w:val="right"/>"#,
        Alignment::Center => r#"<w:jc w:val="center"/>"#,
        Alignment::Distribute | Alignment::Split => r#"<w:jc w:val="distribute"/>"#,
    });
    if matches!(ps.head_type, HeadType::Outline) {
        let _ = write!(s, r#"<w:outlineLvl w:val="{}"/>"#, ps.para_level.min(8));
    }
    s
}

fn border_fill_xml(bf: &BorderFill) -> String {
    let mut s = String::from("<w:tcBorders>");
    for (tag, line) in [("top", &bf.borders[2]), ("left", &bf.borders[0]), ("bottom", &bf.borders[3]), ("right", &bf.borders[1])] {
        s.push_str(&border_line_xml(tag, line));
    }
    s.push_str("</w:tcBorders>");
    if let Some(solid) = &bf.fill.solid {
        let c = solid.background_color;
        if c != 0xFFFF_FFFF && c & 0x00FF_FFFF != 0x00FF_FFFF {
            let _ = write!(s, r#"<w:shd w:val="clear" w:color="auto" w:fill="{}"/>"#, rgb(c));
        }
    }
    s
}

fn border_line_xml(tag: &str, line: &BorderLine) -> String {
    let val = match line.line_type {
        BorderLineType::None => return format!(r#"<w:{tag} w:val="nil"/>"#),
        BorderLineType::Solid => "single",
        BorderLineType::Dash | BorderLineType::LongDash => "dashed",
        BorderLineType::Dot | BorderLineType::Circle => "dotted",
        BorderLineType::DashDot => "dotDash",
        BorderLineType::DashDotDot => "dotDotDash",
        BorderLineType::Double => "double",
        BorderLineType::ThinThickDouble => "thinThickSmallGap",
        BorderLineType::ThickThinDouble => "thickThinSmallGap",
        BorderLineType::ThinThickThinTriple => "triple",
        BorderLineType::Wave => "wave",
        BorderLineType::DoubleWave => "doubleWave",
        BorderLineType::Thick3D | BorderLineType::Thin3D => "threeDEmboss",
        BorderLineType::Thick3DReverse | BorderLineType::Thin3DReverse => "threeDEngrave",
    };
    // 한글 선 굵기 번호 → mm → 1/8 pt
    const MM: [f64; 16] = [0.1, 0.12, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5, 0.6, 0.7, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0];
    let mm = MM[(line.width as usize).min(15)];
    let sz = ((mm * 72.0 / 25.4) * 8.0).round().clamp(2.0, 96.0) as u32;
    format!(r#"<w:{tag} w:val="{val}" w:sz="{sz}" w:space="0" w:color="{}"/>"#, rgb(line.color))
}

/// 표 열 너비(HWPUNIT). 한 칸짜리 셀에서 먼저 정하고, 모르는 열은 병합 셀 폭을 나눠 채운다.
fn column_widths(t: &Table) -> Vec<u32> {
    let cols = t.col_count as usize;
    let mut w: Vec<Option<u32>> = vec![None; cols];
    for c in &t.cells {
        if c.col_span <= 1 && (c.col as usize) < cols && c.width > 0 {
            let e = &mut w[c.col as usize];
            *e = Some(e.map_or(c.width, |x| x.max(c.width)));
        }
    }
    for c in &t.cells {
        let (c0, span) = (c.col as usize, c.col_span.max(1) as usize);
        if span <= 1 || c0 + span > cols {
            continue;
        }
        let known: u32 = w[c0..c0 + span].iter().flatten().sum();
        let missing = w[c0..c0 + span].iter().filter(|x| x.is_none()).count() as u32;
        if missing > 0 && c.width > known {
            let each = (c.width - known) / missing;
            for x in &mut w[c0..c0 + span] {
                if x.is_none() {
                    *x = Some(each);
                }
            }
        }
    }
    let fallback = t.common.width.max(1) / cols as u32;
    w.into_iter().map(|x| x.unwrap_or(fallback).max(200)).collect()
}

fn rgb(c: u32) -> String {
    format!("{:02X}{:02X}{:02X}", c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF)
}

fn sniff_ext(d: &[u8]) -> Option<&'static str> {
    Some(match d {
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xFF, 0xD8, ..] => "jpeg",
        [b'G', b'I', b'F', ..] => "gif",
        [b'B', b'M', ..] => "bmp",
        [0x01, 0x00, 0x00, 0x00, ..] => "emf",
        [0xD7, 0xCD, 0xC6, 0x9A, ..] => "wmf",
        [0x01 | 0x02, 0x00, 0x09, 0x00, ..] => "wmf",
        [b'I', b'I', 42, 0, ..] | [b'M', b'M', 0, 42, ..] => "tiff",
        _ => return None,
    })
}

/// 표준 WMF 앞에 Aldus 크기 머리글(22바이트)을 붙인다. 경계는 SetWindowOrg/SetWindowExt 레코드에서 읽는다.
fn add_wmf_placeable_header(wmf: &[u8]) -> Option<Vec<u8>> {
    let u16_at = |i: usize| wmf.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let i16_at = |i: usize| u16_at(i).map(|v| v as i16);
    let header_words = u16_at(2)? as usize;
    let (mut org, mut ext) = ((0i16, 0i16), None::<(i16, i16)>);
    let mut pos = header_words * 2;
    while pos + 6 <= wmf.len() {
        let size_words = u32::from_le_bytes(wmf.get(pos..pos + 4)?.try_into().ok()?) as usize;
        let func = u16_at(pos + 4)?;
        match func {
            // 인자는 역순(y, x)으로 저장된다
            0x020B => org = (i16_at(pos + 8)?, i16_at(pos + 6)?),
            0x020C => ext = Some((i16_at(pos + 8)?, i16_at(pos + 6)?)),
            0x0000 => break,
            _ => {}
        }
        if size_words < 3 {
            break;
        }
        pos += size_words * 2;
    }
    let (w, h) = ext?;
    let (left, top) = org;
    let (right, bottom) = (left.saturating_add(w), top.saturating_add(h));
    let mut hdr: Vec<u16> = vec![0xCDD7, 0x9AC6, 0, left as u16, top as u16, right as u16, bottom as u16, 1440, 0, 0];
    let checksum = hdr.iter().fold(0u16, |a, &b| a ^ b);
    hdr.push(checksum);
    let mut out: Vec<u8> = hdr.iter().flat_map(|v| v.to_le_bytes()).collect();
    out.extend_from_slice(wmf);
    Some(out)
}

fn styles_xml(doc: &Document) -> String {
    // 기본 글꼴: '바탕글' 스타일의 글자 모양 (없으면 맑은 고딕 10pt)
    let base = doc.doc_info.styles.first().map(|st| st.char_shape_id as usize).unwrap_or(0);
    let (font, size) = doc
        .doc_info
        .char_shapes
        .get(base)
        .map(|cs| {
            (
                font_name(doc, 0, cs.font_ids[0]).unwrap_or("맑은 고딕").to_string(),
                (cs.base_size / 50).max(2),
            )
        })
        .unwrap_or(("맑은 고딕".into(), 20));
    let f = esc(&font);
    format!(
        r#"{XML_DECL}<w:styles {NS}><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="{f}" w:hAnsi="{f}" w:eastAsia="{f}" w:cs="{f}"/><w:sz w:val="{size}"/><w:szCs w:val="{size}"/><w:lang w:val="ko-KR" w:eastAsia="ko-KR"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style><w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style></w:styles>"#
    )
}

fn rel_xml(r: &Rel) -> String {
    format!(
        r#"<Relationship Id="{}" Type="{}" Target="{}"{}/>"#,
        r.id,
        r.kind,
        esc(&r.target),
        if r.external { r#" TargetMode="External""# } else { "" }
    )
}

fn rels_xml(rels: &[Rel]) -> String {
    let mut s = format!(r#"{XML_DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for r in rels {
        s.push_str(&rel_xml(r));
    }
    s.push_str("</Relationships>");
    s
}

fn zip_files(files: Vec<(String, Vec<u8>)>) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in files {
        zip.start_file(name, opts).map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // XML 1.0 에 쓸 수 없는 문자
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            '\u{FFFE}' | '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}
