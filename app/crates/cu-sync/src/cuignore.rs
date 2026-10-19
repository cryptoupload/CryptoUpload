// SPDX-License-Identifier: GPL-3.0-or-later
use std::str::FromStr;

struct Rule {
    negate: bool,
    dir_only: bool,
    anchored: bool,
    segments: Vec<Segment>,
}

enum Segment {
    DoubleStar,
    Glob(Vec<Tok>),
}

#[derive(Clone, Copy)]
enum Tok {
    Char(char),
    Star,
    Question,
}

#[derive(Default)]
pub struct CuIgnore {
    rules: Vec<Rule>,
}

impl CuIgnore {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn parse(content: &str) -> Self {
        let rules = content.lines().filter_map(compile_line).collect();
        Self { rules }
    }

    #[must_use]
    pub fn matched(&self, rel_path: &str, is_dir: bool) -> bool {
        let path_segments: Vec<&str> = rel_path.split('/').filter(|s| !s.is_empty()).collect();
        let mut ignored = false;
        for rule in &self.rules {
            if rule.dir_only && !is_dir {
                continue;
            }
            if rule_matches(rule, &path_segments) {
                ignored = !rule.negate;
            }
        }
        ignored
    }
}

fn rule_matches(rule: &Rule, path: &[&str]) -> bool {
    if rule.anchored {
        segments_match(&rule.segments, path)
    } else {
        (0..=path.len()).any(|i| segments_match(&rule.segments, &path[i..]))
    }
}

fn segments_match(pattern: &[Segment], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((Segment::DoubleStar, rest)) => {
            (0..=path.len()).any(|i| segments_match(rest, &path[i..]))
        }
        Some((Segment::Glob(toks), rest)) => match path.split_first() {
            Some((first, path_rest)) => {
                seg_match(toks, &first.chars().collect::<Vec<_>>())
                    && segments_match(rest, path_rest)
            }
            None => false,
        },
    }
}

fn seg_match(pat: &[Tok], s: &[char]) -> bool {
    match pat.split_first() {
        None => s.is_empty(),
        Some((Tok::Star, rest)) => (0..=s.len()).any(|i| seg_match(rest, &s[i..])),
        Some((Tok::Question, rest)) => !s.is_empty() && seg_match(rest, &s[1..]),
        Some((Tok::Char(c), rest)) => s.first() == Some(c) && seg_match(rest, &s[1..]),
    }
}

fn compile_line(raw: &str) -> Option<Rule> {
    let line = trim_trailing_unescaped_space(raw);
    if line.is_empty() {
        return None;
    }
    let mut chars = line.chars().peekable();
    if chars.peek() == Some(&'#') {
        return None;
    }

    let negate = if chars.peek() == Some(&'!') {
        chars.next();
        true
    } else {
        false
    };

    let mut body: String = chars.collect();
    if body.is_empty() {
        return None;
    }

    let dir_only = ends_with_unescaped_slash(&body);
    if dir_only {
        body.pop();
    }
    if body.is_empty() {
        return None;
    }

    let anchored = body.starts_with('/') || body[..body.len() - 1].contains('/');
    let body = body.strip_prefix('/').unwrap_or(&body);
    if body.is_empty() {
        return None;
    }

    let segments: Vec<Segment> = body.split('/').map(compile_segment).collect();

    Some(Rule {
        negate,
        dir_only,
        anchored,
        segments,
    })
}

fn compile_segment(seg: &str) -> Segment {
    if seg == "**" {
        return Segment::DoubleStar;
    }
    let mut toks = Vec::with_capacity(seg.len());
    let mut chars = seg.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(escaped) = chars.next() {
                    toks.push(Tok::Char(escaped));
                }
            }
            '*' => toks.push(Tok::Star),
            '?' => toks.push(Tok::Question),
            other => toks.push(Tok::Char(other)),
        }
    }
    Segment::Glob(toks)
}

fn ends_with_unescaped_slash(s: &str) -> bool {
    if !s.ends_with('/') {
        return false;
    }
    let before_slash = &s[..s.len() - 1];
    let backslashes = before_slash
        .chars()
        .rev()
        .take_while(|&c| c == '\\')
        .count();
    backslashes % 2 == 0
}

fn trim_trailing_unescaped_space(s: &str) -> String {
    let mut out = s.to_string();
    loop {
        if !out.ends_with(' ') {
            break;
        }
        let before = &out[..out.len() - 1];
        let backslashes = before.chars().rev().take_while(|&c| c == '\\').count();
        if backslashes % 2 == 1 {
            out = format!("{}{}", &before[..before.len() - 1], " ");
            break;
        }
        out.pop();
    }
    out
}

impl FromStr for CuIgnore {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::parse(s))
    }
}
