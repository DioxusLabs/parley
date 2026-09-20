// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The fixture file format: one line per fact, `<tag> <fields...>`.
//!
//! ```text
//! source css/css-text/white-space/pre-wrap-001.html
//! block 1 html > body > div
//! size 79.984375 40
//! lang
//! font Ahem Ahem.ttf
//! container
//!   direction ltr
//!   font-family Ahem
//!   ...
//! style 0
//!   font-family Ahem
//!   ...
//! run 0 <style> <lang>
//!   "XX    XX"
//! char <run> <index> <x> <y> <width> <height>
//! ```
//!
//! Run text is written quoted and escaped ([`escape`]) so that a fixture is one fact
//! per line, no line ends in white space, and white space and invisible characters —
//! which is what much of this corpus is about — are visible in a diff.

use std::fmt::Write as _;

use crate::{Case, CharRect, FontFace, Run, StyleMap};

/// An error parsing a fixture file.
#[derive(Debug)]
pub struct ParseError {
    line: usize,
    message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

impl Case {
    /// Serializes the case.
    #[must_use]
    pub fn write(&self) -> String {
        let mut out = String::new();
        writeln!(out, "source {}", self.source).unwrap();
        writeln!(out, "block {} {}", self.block, self.path).unwrap();
        writeln!(out, "size {} {}", self.width, self.height).unwrap();
        writeln!(out, "lang{}", optional(&self.lang)).unwrap();
        for font in &self.fonts {
            writeln!(out, "font {} {}", escape(&font.family), font.file).unwrap();
        }
        writeln!(out, "container").unwrap();
        write_style(&mut out, &self.container);
        for (index, style) in self.styles.iter().enumerate() {
            writeln!(out, "style {index}").unwrap();
            write_style(&mut out, style);
        }
        for (index, run) in self.runs.iter().enumerate() {
            writeln!(out, "run {} {}{}", index, run.style, optional(&run.lang)).unwrap();
            writeln!(out, "  \"{}\"", escape(&run.text)).unwrap();
        }
        for c in &self.chars {
            writeln!(
                out,
                "char {} {} {} {} {} {}",
                c.run, c.index, c.x, c.y, c.width, c.height
            )
            .unwrap();
        }
        out
    }

    /// Parses a fixture written by [`Case::write`].
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut case = Self {
            source: String::new(),
            block: 0,
            path: String::new(),
            width: 0.0,
            height: 0.0,
            lang: String::new(),
            fonts: Vec::new(),
            container: StyleMap::new(),
            styles: Vec::new(),
            runs: Vec::new(),
            chars: Vec::new(),
        };

        enum Section {
            None,
            Container,
            Style(usize),
            Run(usize),
        }
        let mut section = Section::None;

        for (number, line) in text.lines().enumerate() {
            let number = number + 1;
            let error = |message: String| ParseError {
                line: number,
                message,
            };
            if let Some(indented) = line.strip_prefix("  ") {
                match section {
                    Section::None => {
                        return Err(error("indented line outside a section".into()));
                    }
                    Section::Container => {
                        let (name, value) = split_property(indented);
                        case.container.insert(name.to_string(), value.to_string());
                    }
                    Section::Style(index) => {
                        let (name, value) = split_property(indented);
                        case.styles[index].insert(name.to_string(), value.to_string());
                    }
                    Section::Run(index) => {
                        let quoted = indented
                            .strip_prefix('"')
                            .and_then(|text| text.strip_suffix('"'))
                            .ok_or_else(|| error("run text must be quoted".into()))?;
                        case.runs[index].text = unescape(quoted).map_err(&error)?;
                    }
                }
                continue;
            }
            section = Section::None;
            let (tag, rest) = line.split_once(' ').unwrap_or((line, ""));
            match tag {
                "source" => case.source = rest.to_string(),
                "block" => {
                    let (index, path) = rest.split_once(' ').unwrap_or((rest, ""));
                    case.block = index
                        .parse()
                        .map_err(|_| error(format!("bad block index {index:?}")))?;
                    case.path = path.to_string();
                }
                "size" => {
                    let mut fields = rest.split(' ');
                    case.width = parse_field(&mut fields).map_err(&error)?;
                    case.height = parse_field(&mut fields).map_err(&error)?;
                }
                "lang" => case.lang = rest.to_string(),
                "font" => {
                    let (family, file) = rest
                        .rsplit_once(' ')
                        .ok_or_else(|| error("expected `font <family> <file>`".into()))?;
                    case.fonts.push(FontFace {
                        family: unescape(family).map_err(&error)?,
                        file: file.to_string(),
                    });
                }
                "container" => section = Section::Container,
                "style" => {
                    let index: usize = rest
                        .parse()
                        .map_err(|_| error(format!("bad style index {rest:?}")))?;
                    if index != case.styles.len() {
                        return Err(error(format!(
                            "style {index} out of order (expected {})",
                            case.styles.len()
                        )));
                    }
                    case.styles.push(StyleMap::new());
                    section = Section::Style(index);
                }
                "run" => {
                    let mut fields = rest.splitn(3, ' ');
                    let index: usize = parse_field(&mut fields).map_err(&error)?;
                    let style: usize = parse_field(&mut fields).map_err(&error)?;
                    let lang = fields.next().unwrap_or("").to_string();
                    if index != case.runs.len() {
                        return Err(error(format!(
                            "run {index} out of order (expected {})",
                            case.runs.len()
                        )));
                    }
                    if style >= case.styles.len() {
                        return Err(error(format!("run {index} references style {style}")));
                    }
                    case.runs.push(Run {
                        text: String::new(),
                        lang,
                        style,
                    });
                    section = Section::Run(index);
                }
                "char" => {
                    let mut fields = rest.split(' ');
                    let run = parse_field(&mut fields).map_err(&error)?;
                    let index = parse_field(&mut fields).map_err(&error)?;
                    let x = parse_field(&mut fields).map_err(&error)?;
                    let y = parse_field(&mut fields).map_err(&error)?;
                    let width = parse_field(&mut fields).map_err(&error)?;
                    let height = parse_field(&mut fields).map_err(&error)?;
                    case.chars.push(CharRect {
                        run,
                        index,
                        x,
                        y,
                        width,
                        height,
                    });
                }
                "" => {}
                other => return Err(error(format!("unknown tag {other:?}"))),
            }
        }
        Ok(case)
    }
}

/// A trailing field that is omitted (with its separating space) when empty.
fn optional(field: &str) -> String {
    if field.is_empty() {
        String::new()
    } else {
        format!(" {field}")
    }
}

/// Writes the properties of `style` with a value; a missing property reads back as an
/// empty value, so empty ones need not be written.
fn write_style(out: &mut String, style: &StyleMap) {
    for (name, value) in style {
        if !value.is_empty() {
            writeln!(out, "  {name} {value}").unwrap();
        }
    }
}

fn split_property(line: &str) -> (&str, &str) {
    line.split_once(' ').unwrap_or((line, ""))
}

fn parse_field<'a, T: std::str::FromStr>(
    fields: &mut impl Iterator<Item = &'a str>,
) -> Result<T, String> {
    let field = fields.next().ok_or("missing field")?;
    field.parse().map_err(|_| format!("bad field {field:?}"))
}

/// Whether `c` is written as a `\u{...}` escape rather than literally.
///
/// Everything invisible or ambiguous on a terminal: controls, format characters (zero
/// width space/joiner/non-joiner, soft hyphen, bidi controls, word joiner, BOM), and
/// every white space character other than the plain ASCII space.
fn needs_escape(c: char) -> bool {
    c.is_control()
        || (c.is_whitespace() && c != ' ')
        || matches!(c,
            '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}' | '\u{1160}' | '\u{17B4}'
            | '\u{17B5}' | '\u{180B}'..='\u{180F}' | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{3164}' | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}' | '\u{FFA0}' | '\u{E0100}'..='\u{E01EF}')
}

/// Escapes `text` onto one line: `\\`, `\"`, `\n`, `\r`, `\t`, `\u{XXXX}` (see
/// [`needs_escape`]).
#[must_use]
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if needs_escape(c) => write!(out, "\\u{{{:04X}}}", u32::from(c)).unwrap(),
            c => out.push(c),
        }
    }
    out
}

/// Inverts [`escape`].
pub(crate) fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('u') => {
                if chars.next() != Some('{') {
                    return Err("expected `{` after `\\u`".into());
                }
                let hex: String = chars.by_ref().take_while(|&c| c != '}').collect();
                let code = u32::from_str_radix(&hex, 16)
                    .map_err(|_| format!("bad unicode escape {hex:?}"))?;
                out.push(char::from_u32(code).ok_or_else(|| format!("invalid code point {hex}"))?);
            }
            other => return Err(format!("bad escape \\{}", other.unwrap_or(' '))),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_round_trips() {
        let text = "a b\tc\nd\\e\u{200B}f\u{00A0}g\u{3000}日本\"";
        let escaped = escape(text);
        assert_eq!(
            escaped,
            "a b\\tc\\nd\\\\e\\u{200B}f\\u{00A0}g\\u{3000}日本\\\""
        );
        assert_eq!(unescape(&escaped).unwrap(), text);
    }

    #[test]
    fn case_round_trips() {
        let mut container = StyleMap::new();
        container.insert("direction".into(), "ltr".into());
        container.insert("font-family".into(), "Ahem, \"Times New Roman\"".into());
        let mut style = StyleMap::new();
        style.insert("font-size".into(), "20px".into());
        let case = Case {
            source: "css/css-text/white-space/pre-wrap-001.html".into(),
            block: 1,
            path: "html > body > div".into(),
            width: 79.984375,
            height: 40.0,
            lang: "en".into(),
            fonts: vec![FontFace {
                family: "Ahem".into(),
                file: "Ahem.ttf".into(),
            }],
            container,
            styles: vec![style],
            runs: vec![
                Run {
                    text: "XX    XX".into(),
                    lang: "en-GB".into(),
                    style: 0,
                },
                Run {
                    text: "\n".into(),
                    lang: String::new(),
                    style: 0,
                },
            ],
            chars: vec![CharRect {
                run: 0,
                index: 1,
                x: 19.984375,
                y: 0.0,
                width: 20.0,
                height: 20.0,
            }],
        };
        let text = case.write();
        assert_eq!(Case::parse(&text).unwrap(), case, "{text}");
    }
}
