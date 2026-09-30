//! 한글 수식 스크립트 → LaTeX.
//!
//! 한글 수식은 LaTeX 가 아니라 한컴 고유 문법(`a over b`, `sqrt {x}`, `sum _{i=1} ^{n}` …)이다.
//! rhwp 의 수식 파서(`renderer::equation::parser`)가 만든 트리를 LaTeX 로 옮긴다.
//! 출력은 KaTeX/MathJax 가 렌더할 수 있는 범위(amsmath 수준)로 제한한다.

use rhwp::renderer::equation::ast::{EqNode, MatrixStyle, PileAlign, SpaceKind};
use rhwp::renderer::equation::symbols::{DecoKind, FontStyleKind};

use crate::equation::{self, is_tall};

/// rhwp 텍스트/Markdown 추출기가 수식 스크립트를 감싸는 표식 (패치 `rhwp-hwp2pdf.patch`).
pub const EQ_START: char = '\u{FFF9}';
pub const EQ_END: char = '\u{FFFB}';

/// 한글 수식 스크립트 하나를 LaTeX 본문(구분자 `$` 없이)으로 바꾼다.
pub fn script_to_latex(script: &str) -> String {
    let ast = equation::parse(script);
    let mut w = Writer::default();
    // 최상위에 줄바꿈(#)이나 칸 맞춤(&)이 있으면 aligned 환경으로 감싼다.
    if equation::is_multiline(&ast) {
        w.align_depth = 1;
        let body = w.node(&ast);
        return format!("\\begin{{aligned}}{}\\end{{aligned}}", body.trim());
    }
    trim_spacing(&tidy(&w.node(&ast)))
}

/// 수식 양끝의 간격 명령(`\,` `\ ` `\quad`)은 문장 속에서 의미가 없으니 떼어 낸다.
fn trim_spacing(s: &str) -> String {
    let mut t = s.trim();
    loop {
        let before = t.len();
        for sp in ["\\,", "\\quad", "\\ "] {
            t = t.strip_prefix(sp).unwrap_or(t).trim_start();
            t = t.strip_suffix(sp).unwrap_or(t).trim_end();
        }
        // tidy 가 끝의 `\ ` 를 `\` 로 줄여 둔 경우는 이미 지워졌다.
        if t.len() == before {
            return t.to_string();
        }
    }
}

/// 추출 텍스트 안의 수식 표식을 LaTeX 로 바꾼다.
/// 수식만 있는 줄은 `$$…$$`(블록), 문장 속 수식은 `$…$`(인라인)로 쓴다.
/// `in_markdown` 이면 표 칸을 깨뜨리는 `|` 를 `\vert` 로 바꾼다.
pub fn replace_markers(text: &str, in_markdown: bool) -> String {
    if !text.contains(EQ_START) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 64);
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&replace_line(line, in_markdown));
    }
    out
}

fn replace_line(line: &str, in_markdown: bool) -> String {
    if !line.contains(EQ_START) {
        return line.to_string();
    }
    let trimmed = line.trim();
    let display = trimmed.starts_with(EQ_START)
        && trimmed.ends_with(EQ_END)
        && trimmed.matches(EQ_START).count() == 1;

    let mut out = String::new();
    let mut rest = line;
    while let Some(start) = rest.find(EQ_START) {
        out.push_str(&rest[..start]);
        let after = &rest[start + EQ_START.len_utf8()..];
        let Some(end) = after.find(EQ_END) else {
            out.push_str(after);
            return out;
        };
        // Markdown 표 칸 이스케이프(`\|`)가 스크립트 안에 들어가 있으면 되돌린다.
        let script = after[..end].replace("\\|", "|");
        let mut latex = script_to_latex(&script);
        if in_markdown && latex.contains('|') {
            latex = latex.replace("\\|", "\\Vert ").replace('|', "\\vert ");
        }
        if display {
            out.push_str(&format!("$${latex}$$"));
        } else {
            out.push_str(&format!("${latex}$"));
        }
        rest = &after[end + EQ_END.len_utf8()..];
    }
    out.push_str(rest);
    out
}

#[derive(Default)]
struct Writer {
    /// &·\\ 를 쓸 수 있는 환경(aligned, matrix, cases …) 안에 있는지
    align_depth: usize,
}

impl Writer {
    fn node(&mut self, n: &EqNode) -> String {
        match n {
            EqNode::Row(children) => {
                let mut out = String::new();
                for c in children {
                    let t = self.node(c);
                    push_token(&mut out, &t);
                }
                out
            }
            EqNode::Text(s) => text(s),
            EqNode::Number(s) => s.clone(),
            EqNode::Symbol(s) => symbol(s),
            EqNode::MathSymbol(s) => math_symbol(s),
            EqNode::Function(name) => function(name),
            EqNode::Fraction { numer, denom } => {
                format!("\\frac{{{}}}{{{}}}", self.inner(numer), self.inner(denom))
            }
            EqNode::Atop { top, bottom } => {
                format!("\\genfrac{{}}{{}}{{0pt}}{{}}{{{}}}{{{}}}", self.inner(top), self.inner(bottom))
            }
            EqNode::Sqrt { index, body } => match index {
                Some(i) => format!("\\sqrt[{}]{{{}}}", self.inner(i), self.inner(body)),
                None => format!("\\sqrt{{{}}}", self.inner(body)),
            },
            EqNode::Superscript { base, sup } => {
                format!("{}^{{{}}}", self.base(base), self.inner(sup))
            }
            EqNode::Subscript { base, sub } => {
                format!("{}_{{{}}}", self.base(base), self.inner(sub))
            }
            EqNode::SubSup { base, sub, sup } => format!(
                "{}_{{{}}}^{{{}}}",
                self.base(base),
                self.inner(sub),
                self.inner(sup)
            ),
            EqNode::BigOp { symbol, sub, sup } => {
                let mut out = big_op(symbol);
                if let Some(s) = sub {
                    out.push_str(&format!("_{{{}}}", self.inner(s)));
                }
                if let Some(s) = sup {
                    out.push_str(&format!("^{{{}}}", self.inner(s)));
                }
                out
            }
            EqNode::Limit { is_upper, sub } => {
                let mut out = if *is_upper { "\\operatorname*{Lim}".to_string() } else { "\\lim".to_string() };
                if let Some(s) = sub {
                    out.push_str(&format!("_{{{}}}", self.inner(s)));
                }
                out
            }
            EqNode::Matrix { rows, style } => {
                let env = match style {
                    MatrixStyle::Plain => "matrix",
                    MatrixStyle::Paren => "pmatrix",
                    MatrixStyle::Bracket => "bmatrix",
                    MatrixStyle::Vert => "vmatrix",
                };
                self.align_depth += 1;
                let body = rows
                    .iter()
                    .map(|row| row.iter().map(|c| self.inner(c)).collect::<Vec<_>>().join(" & "))
                    .collect::<Vec<_>>()
                    .join(" \\\\ ");
                self.align_depth -= 1;
                format!("\\begin{{{env}}}{body}\\end{{{env}}}")
            }
            EqNode::Cases { rows } => {
                self.align_depth += 1;
                let body = rows.iter().map(|r| self.inner(r)).collect::<Vec<_>>().join(" \\\\ ");
                self.align_depth -= 1;
                format!("\\begin{{cases}}{body}\\end{{cases}}")
            }
            EqNode::Pile { rows, align } => {
                let col = match align {
                    PileAlign::Center => "c",
                    PileAlign::Left => "l",
                    PileAlign::Right => "r",
                };
                self.align_depth += 1;
                let body = rows.iter().map(|r| self.inner(r)).collect::<Vec<_>>().join(" \\\\ ");
                self.align_depth -= 1;
                format!("\\begin{{array}}{{{col}}}{body}\\end{{array}}")
            }
            EqNode::EqAlign { rows } => {
                self.align_depth += 1;
                let body = rows
                    .iter()
                    .map(|(l, r)| format!("{} & {}", self.inner(l), self.inner(r)))
                    .collect::<Vec<_>>()
                    .join(" \\\\ ");
                self.align_depth -= 1;
                format!("\\begin{{aligned}}{body}\\end{{aligned}}")
            }
            EqNode::Rel { arrow, over, under } => {
                let arrow = math_symbol(arrow);
                let over = self.inner(over);
                match under {
                    Some(u) => format!("\\overset{{{over}}}{{\\underset{{{}}}{{{arrow}}}}}", self.inner(u)),
                    None => format!("\\overset{{{over}}}{{{arrow}}}"),
                }
            }
            EqNode::Paren { left, right, body } if left == "(" && right == ")" && matches!(body.as_ref(), EqNode::Atop { .. }) => {
                let EqNode::Atop { top, bottom } = body.as_ref() else { unreachable!() };
                format!("\\binom{{{}}}{{{}}}", self.inner(top), self.inner(bottom))
            }
            // 분수·행렬처럼 키 큰 내용이 없으면 \left \right 없이 그냥 괄호로 쓴다 (읽기 쉽게).
            EqNode::Paren { left, right, body } if !is_tall(body) && plain_delim(left) && plain_delim(right) => {
                format!("{}{}{}", plain(left), self.inner(body), plain(right))
            }
            EqNode::Paren { left, right, body } => format!(
                "\\left{} {} \\right{}",
                delimiter(left),
                self.inner(body),
                delimiter(right)
            ),
            EqNode::Decoration { kind, body } => {
                let wide = !is_atom(body);
                let cmd = match kind {
                    DecoKind::Hat => if wide { "\\widehat" } else { "\\hat" },
                    DecoKind::Check => "\\check",
                    DecoKind::Tilde => if wide { "\\widetilde" } else { "\\tilde" },
                    DecoKind::Acute => "\\acute",
                    DecoKind::Grave => "\\grave",
                    DecoKind::Dot => "\\dot",
                    DecoKind::DDot => "\\ddot",
                    DecoKind::Bar => if wide { "\\overline" } else { "\\bar" },
                    DecoKind::Vec => if wide { "\\overrightarrow" } else { "\\vec" },
                    DecoKind::Dyad => "\\overleftrightarrow",
                    DecoKind::Under | DecoKind::Underline => "\\underline",
                    DecoKind::Arch => return format!("\\overset{{\\frown}}{{{}}}", self.inner(body)),
                    DecoKind::Overline => "\\overline",
                    DecoKind::StrikeThrough => "\\cancel",
                };
                format!("{cmd}{{{}}}", self.inner(body))
            }
            EqNode::FontStyle { style, body } => {
                let cmd = match style {
                    FontStyleKind::Roman => "\\mathrm",
                    FontStyleKind::Italic => "\\mathit",
                    FontStyleKind::Bold => "\\mathbf",
                    FontStyleKind::Blackboard => "\\mathbb",
                    FontStyleKind::Calligraphy => "\\mathcal",
                    FontStyleKind::Fraktur => "\\mathfrak",
                    FontStyleKind::SansSerif => "\\mathsf",
                    FontStyleKind::Monospace => "\\mathtt",
                };
                format!("{cmd}{{{}}}", self.inner(body))
            }
            // 색은 의미가 없으므로 버리고 내용만 남긴다.
            EqNode::Color { body, .. } => self.node(body),
            EqNode::Space(kind) => match kind {
                SpaceKind::Normal => "\\ ".to_string(),
                SpaceKind::Thin => "\\,".to_string(),
                SpaceKind::Tab if self.align_depth > 0 => "&".to_string(),
                SpaceKind::Tab => "\\quad".to_string(),
            },
            EqNode::Newline if self.align_depth > 0 => "\\\\".to_string(),
            EqNode::Newline => "\\quad".to_string(),
            EqNode::Quoted(s) => format!("\\text{{{}}}", escape_text(s)),
            EqNode::Empty => String::new(),
        }
    }

    /// 중괄호 안에 들어갈 내용
    fn inner(&mut self, n: &EqNode) -> String {
        tidy(&self.node(n))
    }

    /// 첨자의 밑: 한 덩어리가 아니면 중괄호로 묶는다.
    fn base(&mut self, n: &EqNode) -> String {
        match n {
            EqNode::Empty => "{}".to_string(),
            n if is_atom(n) => self.node(n),
            n => format!("{{{}}}", self.inner(n)),
        }
    }
}

fn plain_delim(d: &str) -> bool {
    matches!(d, "(" | ")" | "[" | "]" | "{" | "}" | "|")
}

fn plain(d: &str) -> &str {
    match d {
        "{" => "\\{",
        "}" => "\\}",
        d => d,
    }
}

fn is_atom(n: &EqNode) -> bool {
    match n {
        EqNode::Text(s) | EqNode::Number(s) => s.chars().count() == 1 || s.chars().all(|c| c.is_ascii_digit() || c == '.'),
        EqNode::Symbol(_) | EqNode::MathSymbol(_) | EqNode::Paren { .. } | EqNode::Function(_) => true,
        EqNode::FontStyle { body, .. } | EqNode::Color { body, .. } => is_atom(body),
        _ => false,
    }
}

/// 토큰을 이어 붙인다. `\alpha` 뒤에 글자가 오면 `\alphax` 가 되지 않도록 공백을 넣는다.
fn push_token(out: &mut String, tok: &str) {
    if tok.is_empty() {
        return;
    }
    let ends_with_cmd = {
        let tail: String = out.chars().rev().take_while(|c| c.is_ascii_alphabetic()).collect();
        !tail.is_empty() && out[..out.len() - tail.len()].ends_with('\\')
    };
    if ends_with_cmd && tok.starts_with(|c: char| c.is_ascii_alphabetic()) {
        out.push(' ');
    } else if !out.is_empty() && !out.ends_with(' ') && needs_space_between(out, tok) {
        out.push(' ');
    }
    out.push_str(tok);
}

/// 사람이 읽기 좋게: 영숫자 토큰끼리는 띄운다 (`2 x` 가 아니라 `2x` 로 붙으면 의미가 같아도 헷갈린다 — 둘 다 LaTeX 로는 같다).
fn needs_space_between(prev: &str, next: &str) -> bool {
    let p = prev.chars().last().unwrap_or(' ');
    let n = next.chars().next().unwrap_or(' ');
    (p.is_alphanumeric() && n.is_alphanumeric()) || (n == '\\' && p.is_alphanumeric())
}

/// 공백을 하나로 모으고 양끝을 다듬는다. 끝에 남은 `\ `(공백 명령)은 뜻이 없으니 버린다.
fn tidy(s: &str) -> String {
    let mut out = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let trailing = out.chars().rev().take_while(|&c| c == '\\').count();
    if trailing % 2 == 1 {
        out.pop();
        out.truncate(out.trim_end().len());
    }
    out
}

fn text(s: &str) -> String {
    // 한글 등 비 ASCII 글자는 수학 이탤릭이 아니라 본문 글자다.
    if s.chars().any(|c| !c.is_ascii() && c.is_alphabetic()) {
        format!("\\text{{{}}}", escape_text(s))
    } else {
        s.chars()
            .map(|c| match c {
                '#' | '$' | '%' | '&' | '_' | '{' | '}' => format!("\\{c}"),
                '\\' => "\\backslash ".to_string(),
                '~' => "\\sim ".to_string(),
                '^' => "\\hat{}".to_string(),
                c => math_symbol(&c.to_string()),
            })
            .collect()
    }
}

fn escape_text(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '#' | '$' | '%' | '&' | '_' | '{' | '}' => format!("\\{c}"),
            '\\' => "\\textbackslash{}".to_string(),
            '~' => "\\textasciitilde{}".to_string(),
            '^' => "\\textasciicircum{}".to_string(),
            c => c.to_string(),
        })
        .collect()
}

fn symbol(s: &str) -> String {
    match s {
        "{" => "\\{".into(),
        "}" => "\\}".into(),
        "#" | "$" | "%" | "&" | "_" => format!("\\{s}"),
        "\\" => "\\backslash".into(),
        "~" => "\\sim".into(),
        "'" => "'".into(),
        _ => math_symbol(s),
    }
}

fn function(name: &str) -> String {
    const KNOWN: &[&str] = &[
        "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh",
        "tanh", "coth", "log", "ln", "lg", "exp", "det", "dim", "gcd", "hom", "ker", "deg", "arg",
        "max", "min", "sup", "inf", "lim", "liminf", "limsup", "Pr",
    ];
    if KNOWN.contains(&name) {
        format!("\\{name}")
    } else {
        format!("\\operatorname{{{}}}", escape_text(name))
    }
}

fn big_op(sym: &str) -> String {
    let cmd = match sym {
        "∑" => "\\sum",
        "∏" => "\\prod",
        "∐" => "\\coprod",
        "∫" => "\\int",
        "∬" => "\\iint",
        "∭" => "\\iiint",
        "∮" => "\\oint",
        "∯" => "\\oiint",
        "∰" => "\\oiiint",
        "⋃" | "∪" => "\\bigcup",
        "⋂" | "∩" => "\\bigcap",
        "⨁" | "⊕" => "\\bigoplus",
        "⨂" | "⊗" => "\\bigotimes",
        "⨀" | "⊙" => "\\bigodot",
        "⨄" => "\\biguplus",
        "⋁" | "∨" => "\\bigvee",
        "⋀" | "∧" => "\\bigwedge",
        other => return math_symbol(other),
    };
    cmd.to_string()
}

fn delimiter(d: &str) -> String {
    match d {
        "" | "." => ".".into(),
        "(" | ")" | "[" | "]" | "/" => d.into(),
        "{" => "\\{".into(),
        "}" => "\\}".into(),
        "|" => "|".into(),
        "||" | "‖" => "\\|".into(),
        "<" | "⟨" | "〈" => "\\langle".into(),
        ">" | "⟩" | "〉" => "\\rangle".into(),
        "⌈" => "\\lceil".into(),
        "⌉" => "\\rceil".into(),
        "⌊" => "\\lfloor".into(),
        "⌋" => "\\rfloor".into(),
        "↑" => "\\uparrow".into(),
        "↓" => "\\downarrow".into(),
        _ => ".".into(),
    }
}

/// 유니코드 수학 기호 → LaTeX 명령. 모르는 글자는 그대로 둔다(KaTeX·MathJax·LLM 모두 읽는다).
fn math_symbol(s: &str) -> String {
    let cmd = match s {
        // 그리스 소문자
        "α" => "\\alpha", "β" => "\\beta", "γ" => "\\gamma", "δ" => "\\delta",
        "ε" => "\\varepsilon", "ϵ" => "\\epsilon", "ζ" => "\\zeta", "η" => "\\eta",
        "θ" => "\\theta", "ϑ" => "\\vartheta", "ι" => "\\iota", "κ" => "\\kappa",
        "λ" => "\\lambda", "μ" => "\\mu", "ν" => "\\nu", "ξ" => "\\xi", "ο" => "o",
        "π" => "\\pi", "ϖ" => "\\varpi", "ρ" => "\\rho", "ϱ" => "\\varrho",
        "σ" => "\\sigma", "ς" => "\\varsigma", "τ" => "\\tau", "υ" => "\\upsilon",
        "φ" => "\\varphi", "ϕ" => "\\phi", "χ" => "\\chi", "ψ" => "\\psi", "ω" => "\\omega",
        // 그리스 대문자
        "Γ" => "\\Gamma", "Δ" => "\\Delta", "Θ" => "\\Theta", "Λ" => "\\Lambda",
        "Ξ" => "\\Xi", "Π" => "\\Pi", "Σ" => "\\Sigma", "Υ" => "\\Upsilon",
        "Φ" => "\\Phi", "Ψ" => "\\Psi", "Ω" => "\\Omega",
        "Α" => "A", "Β" => "B", "Ε" => "E", "Ζ" => "Z", "Η" => "H", "Ι" => "I",
        "Κ" => "K", "Μ" => "M", "Ν" => "N", "Ο" => "O", "Ρ" => "P", "Τ" => "T", "Χ" => "X",
        // 연산
        "×" => "\\times", "÷" => "\\div", "±" => "\\pm", "∓" => "\\mp", "·" | "⋅" => "\\cdot",
        "∘" => "\\circ", "•" | "∙" => "\\bullet", "⊕" => "\\oplus", "⊖" => "\\ominus",
        "⊗" => "\\otimes", "⊙" => "\\odot", "∗" => "\\ast", "⋆" | "★" => "\\star",
        "∪" => "\\cup", "∩" => "\\cap", "∧" => "\\wedge", "∨" => "\\vee", "∖" => "\\setminus",
        "−" => "-",
        // 관계
        "≤" | "≦" => "\\le", "≥" | "≧" => "\\ge", "≠" => "\\ne", "≈" => "\\approx",
        "≡" => "\\equiv", "∼" => "\\sim", "≃" => "\\simeq", "≅" => "\\cong", "∝" => "\\propto",
        "≪" => "\\ll", "≫" => "\\gg", "≺" => "\\prec", "≻" => "\\succ", "≐" => "\\doteq",
        "≒" => "\\fallingdotseq", "≓" => "\\risingdotseq",
        "∈" => "\\in", "∉" => "\\notin", "∋" => "\\ni", "⊂" => "\\subset", "⊃" => "\\supset",
        "⊆" => "\\subseteq", "⊇" => "\\supseteq", "⊊" => "\\subsetneq", "⊋" => "\\supsetneq",
        "⊥" => "\\perp", "∥" => "\\parallel", "∣" => "\\mid", "⊢" => "\\vdash", "⊣" => "\\dashv",
        "⊨" => "\\models",
        // 화살표
        "→" => "\\rightarrow", "←" => "\\leftarrow", "↔" => "\\leftrightarrow",
        "⇒" => "\\Rightarrow", "⇐" => "\\Leftarrow", "⇔" => "\\Leftrightarrow",
        "↑" => "\\uparrow", "↓" => "\\downarrow", "↕" => "\\updownarrow",
        "⇑" => "\\Uparrow", "⇓" => "\\Downarrow", "↦" => "\\mapsto",
        "⟶" => "\\longrightarrow", "⟵" => "\\longleftarrow", "⟹" => "\\Longrightarrow",
        "↗" => "\\nearrow", "↘" => "\\searrow", "↖" => "\\nwarrow", "↙" => "\\swarrow",
        "⇄" => "\\rightleftarrows", "⇌" => "\\rightleftharpoons",
        // 기타
        "∞" => "\\infty", "∂" => "\\partial", "∇" => "\\nabla", "∀" => "\\forall",
        "∃" => "\\exists", "∄" => "\\nexists", "¬" => "\\neg", "∅" | "Ø" => "\\emptyset",
        "∠" => "\\angle", "△" => "\\triangle", "□" => "\\square", "◇" => "\\diamond",
        "∴" => "\\therefore", "∵" => "\\because", "…" => "\\ldots", "⋯" => "\\cdots",
        "⋮" => "\\vdots", "⋱" => "\\ddots", "ℓ" => "\\ell", "ℏ" => "\\hbar", "ℜ" => "\\Re",
        "ℑ" => "\\Im", "ℵ" => "\\aleph", "℘" => "\\wp", "√" => "\\surd", "′" => "'",
        "″" => "''", "‴" => "'''", "°" => "^{\\circ}", "℃" => "^{\\circ}\\mathrm{C}",
        "ℝ" => "\\mathbb{R}", "ℤ" => "\\mathbb{Z}", "ℕ" => "\\mathbb{N}", "ℚ" => "\\mathbb{Q}",
        "ℂ" => "\\mathbb{C}", "⌈" => "\\lceil", "⌉" => "\\rceil", "⌊" => "\\lfloor",
        "⌋" => "\\rfloor", "⟨" | "〈" => "\\langle", "⟩" | "〉" => "\\rangle",
        "♠" => "\\spadesuit", "♡" => "\\heartsuit", "♢" => "\\diamondsuit", "♣" => "\\clubsuit",
        "✓" => "\\checkmark", "†" => "\\dagger", "‡" => "\\ddagger", "§" => "\\S",
        "∑" => "\\sum", "∏" => "\\prod", "∫" => "\\int", "∮" => "\\oint",
        "{" => "\\{", "}" => "\\}", "|" => "|",
        other => return other.to_string(),
    };
    // 명령 뒤에 글자가 바로 붙지 않도록 push_token 이 공백을 넣는다.
    cmd.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_forms() {
        assert_eq!(script_to_latex("1 OVER 2"), "\\frac{1}{2}");
        assert_eq!(script_to_latex("{a+b} over {c-d}"), "\\frac{a+b}{c-d}");
        assert_eq!(script_to_latex("sqrt {x}"), "\\sqrt{x}");
        assert_eq!(script_to_latex("x^2"), "x^{2}");
    }

    #[test]
    fn markers_become_dollars() {
        let s = format!("값은 {EQ_START}a over b{EQ_END} 이다");
        assert_eq!(replace_markers(&s, true), "값은 $\\frac{a}{b}$ 이다");
        let s = format!("{EQ_START}x^2{EQ_END}");
        assert_eq!(replace_markers(&s, true), "$$x^{2}$$");
    }

    /// rhwp 수식 샘플 전체를 변환해 target/eq_latex.tsv 로 쓴다 (KaTeX 검증용).
    /// `cargo test --release -- --ignored dump_catalog`
    #[test]
    #[ignore]
    fn dump_catalog() {
        let catalog = concat!(env!("CARGO_MANIFEST_DIR"), "/third_party/rhwp/src/renderer/equation/fixtures/catalog.tsv");
        let mut out = String::new();
        for line in std::fs::read_to_string(catalog).unwrap().lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            let (id, script) = (cols[0], cols[4]);
            out.push_str(&format!("{id}\t{script}\t{}\n", script_to_latex(script)));
        }
        std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/target/eq_latex.tsv"), out).unwrap();
    }

    #[test]
    #[ignore]
    fn show_common() {
        for s in [
            "x_i^2", "x_i ^2", "x_{i}^{2}", "a_{n+1} = a_n + d", "int _{0} ^{1} f(x) dx", "sum _{k=1} ^{n} k^2",
            "lim _{x -> 0} {sin x} over x = 1", "ALPHA + alpha", "f(x) = {x^2 - 1} over {x + 1}",
            "therefore a LEQ b", "angle ABC = 90 DEG", "bar{AB} PERP overline{CD}", "vec a cdot vec b",
            "3 times 4 div 2", "넓이 = 가로 times 세로", "5 rm cm", "x = {-b +- sqrt {b^2 - 4ac}} over {2a}",
            "n CHOOSE k", "{}_{n} C_{r}", "LEFT | x RIGHT | < 1", "log _{2} 8 = 3", "e^{i pi} + 1 = 0",
            "P(A|B)", "a != b", "1 <= x <= 10", "`a` `b`", "\"abc\" + 1", "x prime", "it f(x)",
        ] {
            println!("{s:40} => {}", script_to_latex(s));
        }
    }
}
