//! 한글 수식 스크립트 → Word 수식(OMML, Office Math Markup Language).
//! Word 에서 그림이 아니라 편집 가능한 수식으로 열린다.

use rhwp::renderer::equation::ast::{EqNode, MatrixStyle, PileAlign, SpaceKind};
use rhwp::renderer::equation::symbols::{DecoKind, FontStyleKind};

use crate::equation;

/// `<m:oMath>…</m:oMath>` 한 덩어리를 돌려준다. `half_points` 는 글자 크기(½pt 단위).
pub fn script_to_omml(script: &str, half_points: u32) -> String {
    let ast = equation::parse(script);
    let mut w = Writer {
        wpr: format!(
            r#"<w:rPr><w:rFonts w:ascii="Cambria Math" w:hAnsi="Cambria Math"/><w:sz w:val="{half_points}"/><w:szCs w:val="{half_points}"/></w:rPr>"#
        ),
        ..Default::default()
    };
    let body = if equation::is_multiline(&ast) {
        // 줄바꿈(#)으로 나뉜 여러 줄 수식은 수식 배열로
        w.align = true;
        let rows = split_rows(&ast);
        let mut s = String::from("<m:eqArr>");
        for r in rows {
            s.push_str(&format!("<m:e>{}</m:e>", w.node(&r)));
        }
        s.push_str("</m:eqArr>");
        s
    } else {
        w.node(&ast)
    };
    format!("<m:oMath>{body}</m:oMath>")
}

fn split_rows(n: &EqNode) -> Vec<EqNode> {
    let EqNode::Row(children) = n else { return vec![n.clone()] };
    let mut rows = vec![Vec::new()];
    for c in children {
        if matches!(c, EqNode::Newline) {
            rows.push(Vec::new());
        } else {
            rows.last_mut().unwrap().push(c.clone());
        }
    }
    rows.into_iter().map(EqNode::Row).collect()
}

#[derive(Default)]
struct Writer {
    /// 수식 배열 안: & 가 정렬 기준점이 된다
    align: bool,
    /// 현재 글꼴 스타일 (m:rPr 내용)
    style: Option<&'static str>,
    /// 모든 수식 글자에 붙는 Word 글자 서식 (글꼴·크기)
    wpr: String,
}

impl Writer {
    fn node(&mut self, n: &EqNode) -> String {
        match n {
            EqNode::Row(children) => children.iter().map(|c| self.node(c)).collect(),
            EqNode::Text(s) => {
                // 한글 등 본문 글자는 수학 이탤릭이 아니라 일반 글자로
                if s.chars().any(|c| !c.is_ascii() && c.is_alphabetic()) {
                    self.run_with(s, "<m:nor/>")
                } else {
                    self.run(s)
                }
            }
            EqNode::Number(s) | EqNode::Symbol(s) | EqNode::MathSymbol(s) => self.run(s),
            EqNode::Function(name) => self.run_with(name, "<m:sty m:val=\"p\"/>"),
            EqNode::Fraction { numer, denom } => format!(
                "<m:f><m:num>{}</m:num><m:den>{}</m:den></m:f>",
                self.node(numer),
                self.node(denom)
            ),
            EqNode::Atop { top, bottom } => format!(
                "<m:f><m:fPr><m:type m:val=\"noBar\"/></m:fPr><m:num>{}</m:num><m:den>{}</m:den></m:f>",
                self.node(top),
                self.node(bottom)
            ),
            EqNode::Sqrt { index, body } => match index {
                Some(i) => format!(
                    "<m:rad><m:deg>{}</m:deg><m:e>{}</m:e></m:rad>",
                    self.node(i),
                    self.node(body)
                ),
                None => format!(
                    "<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e>{}</m:e></m:rad>",
                    self.node(body)
                ),
            },
            EqNode::Superscript { base, sup } => format!(
                "<m:sSup><m:e>{}</m:e><m:sup>{}</m:sup></m:sSup>",
                self.node(base),
                self.node(sup)
            ),
            EqNode::Subscript { base, sub } => format!(
                "<m:sSub><m:e>{}</m:e><m:sub>{}</m:sub></m:sSub>",
                self.node(base),
                self.node(sub)
            ),
            EqNode::SubSup { base, sub, sup } => format!(
                "<m:sSubSup><m:e>{}</m:e><m:sub>{}</m:sub><m:sup>{}</m:sup></m:sSubSup>",
                self.node(base),
                self.node(sub),
                self.node(sup)
            ),
            EqNode::BigOp { symbol, sub, sup } => {
                let mut pr = format!("<m:chr m:val=\"{}\"/><m:limLoc m:val=\"undOvr\"/>", esc(symbol));
                if sub.is_none() {
                    pr.push_str("<m:subHide m:val=\"1\"/>");
                }
                if sup.is_none() {
                    pr.push_str("<m:supHide m:val=\"1\"/>");
                }
                let sub = sub.as_ref().map(|s| self.node(s)).unwrap_or_default();
                let sup = sup.as_ref().map(|s| self.node(s)).unwrap_or_default();
                // 한글 수식은 연산 대상을 따로 묶지 않으므로 본체는 비우고 뒤 내용을 이어 쓴다.
                format!("<m:nary><m:naryPr>{pr}</m:naryPr><m:sub>{sub}</m:sub><m:sup>{sup}</m:sup><m:e/></m:nary>")
            }
            EqNode::Limit { is_upper, sub } => {
                let name = self.run_with(if *is_upper { "Lim" } else { "lim" }, "<m:sty m:val=\"p\"/>");
                match sub {
                    Some(s) => format!(
                        "<m:limLow><m:e>{name}</m:e><m:lim>{}</m:lim></m:limLow>",
                        self.node(s)
                    ),
                    None => name,
                }
            }
            EqNode::Matrix { rows, style } => {
                let cols = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
                let mut m = format!(
                    "<m:m><m:mPr><m:mcs><m:mc><m:mcPr><m:count m:val=\"{cols}\"/><m:mcJc m:val=\"center\"/></m:mcPr></m:mc></m:mcs></m:mPr>"
                );
                for row in rows {
                    m.push_str("<m:mr>");
                    for c in 0..cols {
                        let cell = row.get(c).map(|e| self.node(e)).unwrap_or_default();
                        m.push_str(&format!("<m:e>{cell}</m:e>"));
                    }
                    m.push_str("</m:mr>");
                }
                m.push_str("</m:m>");
                match style {
                    MatrixStyle::Plain => m,
                    MatrixStyle::Paren => delimited("(", ")", &m),
                    MatrixStyle::Bracket => delimited("[", "]", &m),
                    MatrixStyle::Vert => delimited("|", "|", &m),
                }
            }
            EqNode::Cases { rows } => {
                let arr = self.eq_array(rows.iter());
                delimited("{", "", &arr)
            }
            EqNode::Pile { rows, align } => {
                let _ = matches!(align, PileAlign::Center);
                self.eq_array(rows.iter())
            }
            EqNode::EqAlign { rows } => {
                let saved = self.align;
                self.align = true;
                let mut s = String::from("<m:eqArr>");
                for (l, r) in rows {
                    s.push_str(&format!("<m:e>{}{}{}</m:e>", self.node(l), self.run("&"), self.node(r)));
                }
                s.push_str("</m:eqArr>");
                self.align = saved;
                s
            }
            EqNode::Rel { arrow, over, under } => {
                let mut s = format!(
                    "<m:limUpp><m:e>{}</m:e><m:lim>{}</m:lim></m:limUpp>",
                    self.run(arrow),
                    self.node(over)
                );
                if let Some(u) = under {
                    s = format!("<m:limLow><m:e>{s}</m:e><m:lim>{}</m:lim></m:limLow>", self.node(u));
                }
                s
            }
            EqNode::Paren { left, right, body } => {
                let inner = self.node(body);
                delimited(&paren_char(left), &paren_char(right), &inner)
            }
            EqNode::Decoration { kind, body } => {
                let inner = self.node(body);
                let accent = match kind {
                    DecoKind::Hat => "\u{0302}",
                    DecoKind::Check => "\u{030C}",
                    DecoKind::Tilde => "\u{0303}",
                    DecoKind::Acute => "\u{0301}",
                    DecoKind::Grave => "\u{0300}",
                    DecoKind::Dot => "\u{0307}",
                    DecoKind::DDot => "\u{0308}",
                    DecoKind::Vec => "\u{20D7}",
                    DecoKind::Dyad => "\u{20E1}",
                    DecoKind::Arch => "\u{0311}",
                    DecoKind::Bar | DecoKind::Overline => {
                        return format!("<m:bar><m:barPr><m:pos m:val=\"top\"/></m:barPr><m:e>{inner}</m:e></m:bar>")
                    }
                    DecoKind::Under | DecoKind::Underline => {
                        return format!("<m:bar><m:barPr><m:pos m:val=\"bot\"/></m:barPr><m:e>{inner}</m:e></m:bar>")
                    }
                    DecoKind::StrikeThrough => {
                        return format!(
                            "<m:borderBox><m:borderBoxPr><m:hideTop m:val=\"1\"/><m:hideBot m:val=\"1\"/><m:hideLeft m:val=\"1\"/><m:hideRight m:val=\"1\"/><m:strikeH m:val=\"1\"/></m:borderBoxPr><m:e>{inner}</m:e></m:borderBox>"
                        )
                    }
                };
                format!("<m:acc><m:accPr><m:chr m:val=\"{accent}\"/></m:accPr><m:e>{inner}</m:e></m:acc>")
            }
            EqNode::FontStyle { style, body } => {
                let saved = self.style;
                self.style = Some(match style {
                    FontStyleKind::Roman => "<m:sty m:val=\"p\"/>",
                    FontStyleKind::Italic => "<m:sty m:val=\"i\"/>",
                    FontStyleKind::Bold => "<m:sty m:val=\"b\"/>",
                    FontStyleKind::Blackboard => "<m:scr m:val=\"double-struck\"/>",
                    FontStyleKind::Calligraphy => "<m:scr m:val=\"script\"/>",
                    FontStyleKind::Fraktur => "<m:scr m:val=\"fraktur\"/>",
                    FontStyleKind::SansSerif => "<m:scr m:val=\"sans-serif\"/>",
                    FontStyleKind::Monospace => "<m:scr m:val=\"monospace\"/>",
                });
                let s = self.node(body);
                self.style = saved;
                s
            }
            EqNode::Color { body, .. } => self.node(body),
            EqNode::Space(kind) => match kind {
                SpaceKind::Normal => self.run(" "),
                SpaceKind::Thin => self.run("\u{2009}"),
                SpaceKind::Tab if self.align => self.run("&"),
                SpaceKind::Tab => self.run("\u{2003}"),
            },
            EqNode::Newline => self.run("\u{2003}"),
            EqNode::Quoted(s) => self.run_with(s, "<m:nor/>"),
            EqNode::Empty => String::new(),
        }
    }

    fn eq_array<'a>(&mut self, rows: impl Iterator<Item = &'a EqNode>) -> String {
        let saved = self.align;
        self.align = true;
        let mut s = String::from("<m:eqArr>");
        for r in rows {
            s.push_str(&format!("<m:e>{}</m:e>", self.node(r)));
        }
        s.push_str("</m:eqArr>");
        self.align = saved;
        s
    }

    fn run(&self, text: &str) -> String {
        match self.style {
            Some(pr) => self.run_with(text, pr),
            None => format!("<m:r>{}<m:t xml:space=\"preserve\">{}</m:t></m:r>", self.wpr, esc(text)),
        }
    }

    fn run_with(&self, text: &str, rpr: &str) -> String {
        format!(
            "<m:r><m:rPr>{rpr}</m:rPr>{}<m:t xml:space=\"preserve\">{}</m:t></m:r>",
            self.wpr,
            esc(text)
        )
    }
}

fn delimited(beg: &str, end: &str, inner: &str) -> String {
    format!(
        "<m:d><m:dPr><m:begChr m:val=\"{}\"/><m:endChr m:val=\"{}\"/></m:dPr><m:e>{inner}</m:e></m:d>",
        esc(beg),
        esc(end)
    )
}

fn paren_char(d: &str) -> String {
    match d {
        "." => String::new(),
        "<" => "⟨".into(),
        ">" => "⟩".into(),
        "||" => "‖".into(),
        d => d.to_string(),
    }
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}
