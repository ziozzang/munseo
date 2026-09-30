//! 한글 수식 스크립트를 rhwp 파서로 읽고, 한글 문법의 의미에 맞게 트리를 다듬는다.
//! LaTeX(`latex.rs`)와 Word 수식 OMML(`omml.rs`)이 같은 결과를 쓴다.
//!
//! rhwp 파서는 화면 렌더링용이라 몇 가지를 한글 수식 편집기와 다르게 묶는다.
//! - `x_i^2` 를 x_{i^2} 로 중첩 → 한글에서는 아래·위첨자 동시 표기
//! - `+-` `!=` `<=` 같은 두 글자 연산자를 기호 두 개로 둠 → ± ≠ ≤
//! - `n CHOOSE k` 를 `n` 과 `( ATOP{빈칸, k} )` 로 나눔 → 이항계수
//! - `rm` `it` `bold` 를 다음 한 토큰에만 적용 → 한글에서는 다음 전환까지 이어지는 모드

use rhwp::renderer::equation::ast::EqNode;
use rhwp::renderer::equation::parser;
use rhwp::renderer::equation::symbols::FontStyleKind;

pub fn parse(script: &str) -> EqNode {
    normalize(parser::parse(script))
}

fn normalize(n: EqNode) -> EqNode {
    use EqNode::*;
    let b = |x: Box<EqNode>| Box::new(normalize(*x));
    let ob = |x: Option<Box<EqNode>>| x.map(|x| Box::new(normalize(*x)));
    match n {
        Row(children) => Row(normalize_row(children.into_iter().map(normalize).collect())),
        Symbol(s) => match two_char_operator(&s) {
            Some(op) => MathSymbol(op.to_string()),
            None => Symbol(s),
        },
        Subscript { base, sub } => match normalize(*sub) {
            Superscript { base: s, sup } => SubSup { base: b(base), sub: s, sup },
            sub => Subscript { base: b(base), sub: Box::new(sub) },
        },
        Superscript { base, sup } => Superscript { base: b(base), sup: b(sup) },
        SubSup { base, sub, sup } => SubSup { base: b(base), sub: b(sub), sup: b(sup) },
        Fraction { numer, denom } => Fraction { numer: b(numer), denom: b(denom) },
        Atop { top, bottom } => Atop { top: b(top), bottom: b(bottom) },
        Sqrt { index, body } => Sqrt { index: ob(index), body: b(body) },
        BigOp { symbol, sub, sup } => BigOp { symbol, sub: ob(sub), sup: ob(sup) },
        Limit { is_upper, sub } => Limit { is_upper, sub: ob(sub) },
        Matrix { rows, style } => Matrix {
            rows: rows.into_iter().map(|r| r.into_iter().map(normalize).collect()).collect(),
            style,
        },
        Cases { rows } => Cases { rows: rows.into_iter().map(normalize).collect() },
        Pile { rows, align } => Pile { rows: rows.into_iter().map(normalize).collect(), align },
        EqAlign { rows } => EqAlign {
            rows: rows.into_iter().map(|(l, r)| (normalize(l), normalize(r))).collect(),
        },
        Rel { arrow, over, under } => Rel { arrow, over: b(over), under: ob(under) },
        Paren { left, right, body } => Paren { left, right, body: b(body) },
        Decoration { kind, body } => Decoration { kind, body: b(body) },
        FontStyle { style, body } => FontStyle { style, body: b(body) },
        Color { r, g, b: bl, body } => Color { r, g, b: bl, body: b(body) },
        other => other,
    }
}

fn normalize_row(children: Vec<EqNode>) -> Vec<EqNode> {
    use EqNode::*;
    let mut out: Vec<EqNode> = Vec::with_capacity(children.len());
    let mut it = children.into_iter().peekable();
    while let Some(node) = it.next() {
        // 두 글자 연산자
        if let Symbol(a) = &node {
            if let Some(Symbol(bs)) = it.peek() {
                if let Some(op) = compound_operator(a, bs) {
                    it.next();
                    out.push(MathSymbol(op.to_string()));
                    continue;
                }
            }
        }
        // rm / it / bold 모드
        if let FontStyle { style, body } = &node {
            if is_mode_style(*style) {
                let mut run = vec![body.as_ref().clone()];
                while let Some(next) = it.peek() {
                    if is_mode_switch(next) {
                        break;
                    }
                    run.push(it.next().unwrap());
                }
                let run = Row(run);
                out.push(match style {
                    FontStyleKind::Italic => run, // 수식 기본이 이탤릭이다
                    _ => FontStyle { style: *style, body: Box::new(run) },
                });
                continue;
            }
        }
        // n CHOOSE k
        if let Paren { left, right, body } = &node {
            if let Atop { top, bottom } = body.as_ref() {
                if matches!(top.as_ref(), Empty) && left == "(" && right == ")" {
                    if let Some(prev) = out.pop() {
                        out.push(Paren {
                            left: left.clone(),
                            right: right.clone(),
                            body: Box::new(Atop { top: Box::new(prev), bottom: bottom.clone() }),
                        });
                        continue;
                    }
                }
            }
        }
        out.push(node);
    }
    out
}

fn compound_operator(a: &str, b: &str) -> Option<&'static str> {
    Some(match (a, b) {
        ("+", "-") => "±",
        ("-", "+") => "∓",
        ("!", "=") => "≠",
        ("<", "=") => "≤",
        (">", "=") => "≥",
        ("<", "<") => "≪",
        (">", ">") => "≫",
        ("-", ">") => "→",
        ("<", "-") => "←",
        _ => return None,
    })
}

/// 토크나이저가 두 글자 연산자를 한 기호로 넘겨 주는 경우
fn two_char_operator(s: &str) -> Option<&'static str> {
    let mut c = s.chars();
    let (a, b) = (c.next()?, c.next()?);
    if c.next().is_some() {
        return None;
    }
    compound_operator(&a.to_string(), &b.to_string())
}

fn is_mode_style(style: FontStyleKind) -> bool {
    matches!(style, FontStyleKind::Roman | FontStyleKind::Italic | FontStyleKind::Bold)
}

fn is_mode_switch(n: &EqNode) -> bool {
    matches!(n, EqNode::FontStyle { style, .. } if is_mode_style(*style))
}

/// 최상위에 줄바꿈(#)이나 칸 맞춤(&)이 있어 여러 줄 정렬로 써야 하는지
pub fn is_multiline(node: &EqNode) -> bool {
    use rhwp::renderer::equation::ast::SpaceKind;
    match node {
        EqNode::Newline | EqNode::Space(SpaceKind::Tab) => true,
        EqNode::Row(children) => children.iter().any(is_multiline),
        _ => false,
    }
}

/// 분수·행렬처럼 키 큰 내용인지 (괄호 크기 자동 조절이 필요한지)
pub fn is_tall(n: &EqNode) -> bool {
    use EqNode::*;
    match n {
        Fraction { .. } | Atop { .. } | Matrix { .. } | Cases { .. } | Pile { .. }
        | EqAlign { .. } | BigOp { .. } | Rel { .. } => true,
        Row(c) => c.iter().any(is_tall),
        Sqrt { body, .. } => is_tall(body),
        Superscript { base, sup } => is_tall(base) || is_tall(sup),
        Subscript { base, sub } => is_tall(base) || is_tall(sub),
        SubSup { base, sub, sup } => is_tall(base) || is_tall(sub) || is_tall(sup),
        Paren { body, .. } | Decoration { body, .. } | FontStyle { body, .. } | Color { body, .. } => {
            is_tall(body)
        }
        _ => false,
    }
}
