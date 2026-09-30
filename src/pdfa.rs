//! rhwp 가 만든 PDF 를 PDF/A-2u (ISO 19005-2, Level U) 로 맞추는 후처리.
//!
//! rhwp(svg2pdf) 출력은 이미 폰트를 서브셋 임베드하고 ToUnicode CMap 을 달아 두므로
//! 텍스트는 살아 있다. PDF/A 로 인정받는 데 빠진 것은 문서 수준 요소들이다:
//! - 카탈로그의 XMP 메타데이터 스트림 (pdfaid:part=2, conformance=U)
//! - sRGB OutputIntent (DeviceRGB 사용을 정당화)
//! - 트레일러 /ID
//! - Info 사전과 XMP 의 일치
//! - 이미지의 /Interpolate true 금지

use lopdf::{dictionary, Document, Object, ObjectId, Stream, StringFormat};

use crate::icc;

const PRODUCER: &str = concat!("Munseo ", env!("CARGO_PKG_VERSION"), " (rhwp)");

pub fn to_pdfa2u(pdf: &[u8], title: &str) -> Result<Vec<u8>, String> {
    let mut doc = Document::load_mem(pdf).map_err(|e| format!("PDF 읽기 실패: {e}"))?;
    doc.version = "1.7".to_string();

    strip_forbidden_keys(&mut doc);
    map_orphan_glyphs(&mut doc);

    let now = time::OffsetDateTime::now_utc();
    let pdf_date = format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}+00'00'",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    let xmp_date = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );

    // Info 사전 — XMP 와 같은 값만 둔다.
    let info_id = doc.add_object(dictionary! {
        "Title" => pdf_text_string(title),
        "Producer" => pdf_text_string(PRODUCER),
        "Creator" => pdf_text_string(PRODUCER),
        "CreationDate" => Object::String(pdf_date.clone().into_bytes(), StringFormat::Literal),
        "ModDate" => Object::String(pdf_date.into_bytes(), StringFormat::Literal),
    });
    doc.trailer.set("Info", Object::Reference(info_id));

    // XMP 메타데이터 — PDF/A 는 메타데이터 스트림에 필터(압축)를 허용하지 않는다.
    let xmp = xmp_packet(title, &xmp_date);
    let mut meta = Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        xmp.into_bytes(),
    );
    meta.allows_compression = false;
    let meta_id = doc.add_object(meta);

    // sRGB OutputIntent
    let profile = icc::srgb_profile();
    let icc_id = doc.add_object(Stream::new(dictionary! { "N" => 3 }, profile));
    let intent_id = doc.add_object(dictionary! {
        "Type" => "OutputIntent",
        "S" => "GTS_PDFA1",
        "OutputConditionIdentifier" => Object::string_literal("sRGB IEC61966-2.1"),
        "Info" => Object::string_literal("sRGB IEC61966-2.1"),
        "RegistryName" => Object::string_literal("http://www.color.org"),
        "DestOutputProfile" => Object::Reference(icc_id),
    });

    let root_id = catalog_id(&doc)?;
    let catalog = doc
        .get_object_mut(root_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| format!("카탈로그 없음: {e}"))?;
    catalog.set("Metadata", Object::Reference(meta_id));
    catalog.set("OutputIntents", Object::Array(vec![Object::Reference(intent_id)]));
    catalog.set("Lang", Object::string_literal("ko-KR"));

    // 트레일러 /ID — 내용 해시로 결정론적으로 만든다.
    let digest = md5::compute(pdf).0.to_vec();
    doc.trailer.set(
        "ID",
        Object::Array(vec![
            Object::String(digest.clone(), StringFormat::Hexadecimal),
            Object::String(digest, StringFormat::Hexadecimal),
        ]),
    );

    let mut out = Vec::with_capacity(pdf.len() + 8192);
    doc.save_to(&mut out).map_err(|e| format!("PDF 저장 실패: {e}"))?;
    Ok(out)
}

fn catalog_id(doc: &Document) -> Result<ObjectId, String> {
    doc.trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|e| format!("트레일러에 Root 가 없습니다: {e}"))
}

/// PDF/A-2 가 금지하는 키를 제거한다.
fn strip_forbidden_keys(doc: &mut Document) {
    for object in doc.objects.values_mut() {
        let dict = match object {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &mut s.dict,
            _ => continue,
        };
        // 6.2.8: 이미지의 Interpolate 는 true 일 수 없다.
        if dict.get(b"Interpolate").and_then(Object::as_bool).unwrap_or(false) {
            dict.remove(b"Interpolate");
        }
        // 6.2.5: ExtGState 의 TR 금지, TR2 는 Default 만.
        dict.remove(b"TR");
        if dict.get(b"TR2").and_then(Object::as_name).map(|n| n != b"Default").unwrap_or(false) {
            dict.remove(b"TR2");
        }
    }
}

/// 유니코드 연결이 없는 글리프를 U+200B(폭 없는 공백)에 연결한다 (PDF/A-2u 6.2.11.7.2).
///
/// 옛한글 음절(ᄒᆞᆫ)처럼 자모 여러 개를 조각 글리프로 조합해 그리면 svg2pdf 는 첫 글리프에만
/// 원문을 연결하고 나머지 조각은 비워 둔다. 첫 글리프가 음절 전체를 담고 있으므로 조각에
/// 보이지 않는 문자를 달아도 추출되는 글은 그대로다.
fn map_orphan_glyphs(doc: &mut Document) {
    // (ToUnicode 스트림 id, 글꼴이 가진 CID 목록)
    let mut targets: Vec<(ObjectId, Vec<u32>)> = Vec::new();
    for obj in doc.objects.values() {
        let Ok(font) = obj.as_dict() else { continue };
        if font.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Type0".as_slice()) {
            continue;
        }
        let Ok(tu_id) = font.get(b"ToUnicode").and_then(Object::as_reference) else { continue };
        let desc = font
            .get(b"DescendantFonts")
            .and_then(Object::as_array)
            .ok()
            .and_then(|a| a.first())
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok());
        let Some(w) = desc.and_then(|d| d.get(b"W").ok()).and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_array().ok()) else {
            continue;
        };
        targets.push((tu_id, cids_from_w(w)));
    }
    for (tu_id, cids) in targets {
        let Ok(Object::Stream(stream)) = doc.get_object_mut(tu_id) else { continue };
        let Ok(content) = stream.decompressed_content().or_else(|_| Ok::<_, lopdf::Error>(stream.content.clone())) else { continue };
        let text = String::from_utf8_lossy(&content).into_owned();
        let mapped = mapped_cids(&text);
        let missing: Vec<u32> = cids.into_iter().filter(|c| *c != 0 && !mapped.contains(c)).collect();
        if missing.is_empty() {
            continue;
        }
        let mut add = String::new();
        for chunk in missing.chunks(100) {
            add.push_str(&format!("{} beginbfchar\n", chunk.len()));
            for c in chunk {
                add.push_str(&format!("<{c:04X}> <200B>\n"));
            }
            add.push_str("endbfchar\n");
        }
        let Some(pos) = text.find("endcmap") else { continue };
        let mut new_text = text.clone();
        new_text.insert_str(pos, &add);
        stream.dict.remove(b"Filter");
        stream.dict.remove(b"DecodeParms");
        stream.set_content(new_text.into_bytes());
    }
}

/// CIDFont 의 W 배열에서 CID 목록을 뽑는다. 형식: `c [w1 w2 …]` 또는 `c_first c_last w`.
fn cids_from_w(w: &[Object]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < w.len() {
        let Ok(first) = w[i].as_i64() else { break };
        match w.get(i + 1) {
            Some(Object::Array(ws)) => {
                out.extend((0..ws.len() as i64).map(|k| (first + k) as u32));
                i += 2;
            }
            Some(o) => {
                let Ok(last) = o.as_i64() else { break };
                out.extend((first..=last.min(first + 65535)).map(|c| c as u32));
                i += 3;
            }
            None => break,
        }
    }
    out
}

/// ToUnicode CMap 에서 이미 연결된 CID
fn mapped_cids(cmap: &str) -> std::collections::HashSet<u32> {
    let mut set = std::collections::HashSet::new();
    let hex = |s: &str| u32::from_str_radix(s.trim_matches(|c| c == '<' || c == '>'), 16).ok();
    let mut mode = "";
    for line in cmap.lines() {
        let l = line.trim();
        if l.ends_with("beginbfchar") {
            mode = "char";
            continue;
        }
        if l.ends_with("beginbfrange") {
            mode = "range";
            continue;
        }
        if l == "endbfchar" || l == "endbfrange" {
            mode = "";
            continue;
        }
        let parts: Vec<&str> = l.split_whitespace().collect();
        match mode {
            "char" => {
                if let Some(c) = parts.first().and_then(|p| hex(p)) {
                    set.insert(c);
                }
            }
            "range" => {
                if let (Some(a), Some(b)) = (parts.first().and_then(|p| hex(p)), parts.get(1).and_then(|p| hex(p))) {
                    set.extend(a..=b.min(a + 65535));
                }
            }
            _ => {}
        }
    }
    set
}

/// PDF 텍스트 문자열: ASCII 면 그대로, 아니면 UTF-16BE + BOM.
fn pdf_text_string(s: &str) -> Object {
    if s.is_ascii() {
        Object::string_literal(s)
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in s.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        Object::String(bytes, StringFormat::Hexadecimal)
    }
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 에서 쓸 수 없는 제어문자는 버린다.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn xmp_packet(title: &str, date: &str) -> String {
    let title = xml_escape(title);
    let producer = xml_escape(PRODUCER);
    format!(
        r#"<?xpacket begin="{bom}" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:pdf="http://ns.adobe.com/pdf/1.3/">
   <pdfaid:part>2</pdfaid:part>
   <pdfaid:conformance>U</pdfaid:conformance>
   <dc:format>application/pdf</dc:format>
   <dc:title><rdf:Alt><rdf:li xml:lang="x-default">{title}</rdf:li></rdf:Alt></dc:title>
   <xmp:CreateDate>{date}</xmp:CreateDate>
   <xmp:ModifyDate>{date}</xmp:ModifyDate>
   <xmp:MetadataDate>{date}</xmp:MetadataDate>
   <xmp:CreatorTool>{producer}</xmp:CreatorTool>
   <pdf:Producer>{producer}</pdf:Producer>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#,
        bom = '\u{FEFF}'
    )
}
