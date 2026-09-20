// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The Parley side: laying a [`Case`] out the way Chrome laid out the page, and reading
//! per-character rects back out of the result.
//!
//! The case is built with the [`TreeBuilder`], one style span per run, because that is
//! the builder that implements CSS white space processing. The processed text it
//! produces is matched back against the runs' text ([`char_index`]) so that clusters
//! can be traced to the DOM text node — and the code point — Chrome measured.
//!
//! Two Chrome behaviours are modelled here rather than tolerated in the comparison, as
//! `linebreaking_matches_chrome.rs` does: `font-size` is truncated to 1/100 px, and
//! lines are broken against a width 1/64 px (one `LayoutUnit`) wider than the
//! container's.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontique::{Blob, Collection, CollectionOptions, FontInfoOverride, SourceCache};
use parley::{
    Alignment, AlignmentOptions, BaseDirection, CHROMIUM_LINE_BREAK_OVERRIDE, FontContext,
    FontFamily, FontFeature, FontFeatures, FontStyle, FontWeight, FontWidth, IndentOptions,
    Language, Layout, LayoutContext, LineHeight, OverflowWrap, StyleProperty, TextStyle,
    TextWrapMode, TreeBuilder, WhiteSpaceCollapse, WordBreak,
};

use crate::{Case, CharRect, ClusterRect, Run};

/// Chrome positions boxes in `LayoutUnit`s of 1/64 px.
pub const LAYOUT_UNIT: f64 = 1.0 / 64.0;

/// Why a case could not be laid out.
#[derive(Debug)]
pub struct LayoutError(pub String);

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LayoutError {}

/// The fixture corpus' font files, read once each.
#[derive(Debug, Default)]
pub struct FontFiles {
    dir: PathBuf,
    loaded: HashMap<String, Blob<u8>>,
}

impl FontFiles {
    /// Reads fonts from `dir` on demand.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            loaded: HashMap::new(),
        }
    }

    fn get(&mut self, file: &str) -> Result<Blob<u8>, LayoutError> {
        if let Some(blob) = self.loaded.get(file) {
            return Ok(blob.clone());
        }
        let path = self.dir.join(file);
        let bytes = std::fs::read(&path)
            .map_err(|error| LayoutError(format!("reading {}: {error}", path.display())))?;
        let blob = Blob::new(Arc::new(bytes));
        self.loaded.insert(file.to_string(), blob.clone());
        Ok(blob)
    }
}

/// Builds a [`FontContext`] holding exactly the case's fonts, each registered under its
/// `@font-face` family name, with no system fonts.
pub fn font_context(case: &Case, fonts: &mut FontFiles) -> Result<FontContext, LayoutError> {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    for face in &case.fonts {
        let blob = fonts.get(&face.file)?;
        let registered = collection.register_fonts(
            blob,
            Some(FontInfoOverride {
                family_name: Some(&face.family),
                width: None,
                style: None,
                weight: None,
                axes: None,
            }),
        );
        if registered.is_empty() {
            return Err(LayoutError(format!("{} contains no fonts", face.file)));
        }
    }
    Ok(FontContext {
        collection,
        source_cache: SourceCache::default(),
    })
}

/// Parses a computed `<length>` in px.
fn px(value: &str) -> Option<f32> {
    value.strip_suffix("px")?.trim().parse().ok()
}

/// Chrome truncates font sizes to 1/100 px.
fn chromium_quantized_font_size(size: f32) -> f32 {
    (size * 100.0).trunc() / 100.0
}

fn alignment(case: &Case) -> Alignment {
    match case.container_property("text-align") {
        "left" | "-webkit-left" => Alignment::Left,
        "right" | "-webkit-right" => Alignment::Right,
        "center" | "-webkit-center" => Alignment::Center,
        "justify" => Alignment::Justify,
        "end" => Alignment::End,
        _ => Alignment::Start,
    }
}

fn base_direction(case: &Case) -> BaseDirection {
    match case.container_property("direction") {
        "rtl" => BaseDirection::Rtl,
        _ => BaseDirection::Ltr,
    }
}

/// The style properties `run`'s computed style maps to.
fn run_styles<'a>(case: &'a Case, run: &'a Run) -> Result<Vec<StyleProperty<'a, ()>>, LayoutError> {
    let property = |name: &str| case.run_property(run, name);
    let mut styles: Vec<StyleProperty<'a, ()>> = Vec::new();

    styles.push(StyleProperty::FontFamily(FontFamily::Source(
        property("font-family").into(),
    )));
    let font_size = px(property("font-size"))
        .ok_or_else(|| LayoutError(format!("font-size: {}", property("font-size"))))?;
    styles.push(StyleProperty::FontSize(chromium_quantized_font_size(
        font_size,
    )));
    if let Some(weight) = FontWeight::parse_css(property("font-weight")) {
        styles.push(StyleProperty::FontWeight(weight));
    }
    if let Some(style) = FontStyle::parse_css(property("font-style")) {
        styles.push(StyleProperty::FontStyle(style));
    }
    if let Some(width) = FontWidth::parse_css(property("font-stretch")) {
        styles.push(StyleProperty::FontWidth(width));
    }
    match property("line-height") {
        "" | "normal" => styles.push(StyleProperty::LineHeight(LineHeight::MetricsRelative(1.0))),
        value => {
            let height = px(value).ok_or_else(|| LayoutError(format!("line-height: {value}")))?;
            styles.push(StyleProperty::LineHeight(LineHeight::Absolute(height)));
        }
    }
    match property("letter-spacing") {
        "" | "normal" => {}
        value => {
            let spacing =
                px(value).ok_or_else(|| LayoutError(format!("letter-spacing: {value}")))?;
            styles.push(StyleProperty::LetterSpacing(spacing));
        }
    }
    match property("word-spacing") {
        "" | "normal" => {}
        value => {
            let spacing = px(value).ok_or_else(|| LayoutError(format!("word-spacing: {value}")))?;
            styles.push(StyleProperty::WordSpacing(spacing));
        }
    }
    styles.push(StyleProperty::WhiteSpaceCollapse(
        match property("white-space-collapse") {
            "preserve" => WhiteSpaceCollapse::Preserve,
            "preserve-breaks" => WhiteSpaceCollapse::PreserveBreaks,
            _ => WhiteSpaceCollapse::Collapse,
        },
    ));
    styles.push(StyleProperty::TextWrapMode(
        match property("text-wrap-mode") {
            "nowrap" => TextWrapMode::NoWrap,
            _ => TextWrapMode::Wrap,
        },
    ));
    styles.push(StyleProperty::WordBreak(match property("word-break") {
        "break-all" => WordBreak::BreakAll,
        "keep-all" => WordBreak::KeepAll,
        _ => WordBreak::Normal,
    }));
    styles.push(StyleProperty::OverflowWrap(
        match property("overflow-wrap") {
            "anywhere" => OverflowWrap::Anywhere,
            "break-word" => OverflowWrap::BreakWord,
            _ => OverflowWrap::Normal,
        },
    ));

    let mut features: Vec<FontFeature> = Vec::new();
    let mut parse_features = |source: &str| -> Result<(), LayoutError> {
        for feature in FontFeature::parse_css_list(source) {
            features.push(feature.map_err(|error| {
                LayoutError(format!("font-feature-settings {source:?}: {error}"))
            })?);
        }
        Ok(())
    };
    match property("font-feature-settings") {
        "" | "normal" => {}
        value => parse_features(value)?,
    }
    if property("font-kerning") == "none" {
        parse_features("\"kern\" 0")?;
    }
    match property("font-variant-ligatures") {
        "none" => parse_features("\"liga\" 0, \"clig\" 0, \"dlig\" 0, \"hlig\" 0, \"calt\" 0")?,
        "no-common-ligatures" => parse_features("\"liga\" 0, \"clig\" 0")?,
        _ => {}
    }
    if !features.is_empty() {
        styles.push(StyleProperty::FontFeatures(FontFeatures::List(
            features.into(),
        )));
    }
    match property("font-variation-settings") {
        "" | "normal" => {}
        value => styles.push(StyleProperty::FontVariations(value.into())),
    }

    if !run.lang.is_empty()
        && let Ok(language) = Language::parse(&run.lang)
    {
        styles.push(StyleProperty::Locale(Some(language)));
    }
    Ok(styles)
}

/// Parley's layout of a case and the white-space-processed text it is over.
#[derive(Debug)]
pub struct CaseLayout {
    /// The layout.
    pub layout: Layout<()>,
    /// The text [`Self::layout`] indexes: the runs' text after white space processing.
    pub text: String,
}

/// Lays `case` out: one style span per run in a tree builder over the runs' text,
/// broken at the container width and aligned per `text-align`.
pub fn layout(
    case: &Case,
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
) -> Result<CaseLayout, LayoutError> {
    let root_style = TextStyle::default();
    let mut builder: TreeBuilder<'_, ()> = layout_cx.tree_builder(font_cx, 1.0, false, &root_style);
    builder.set_base_direction(base_direction(case));
    builder.set_line_break_override(Some(CHROMIUM_LINE_BREAK_OVERRIDE));

    for run in &case.runs {
        let styles = run_styles(case, run)?;
        builder.push_style_modification_span(&styles);
        builder.push_text(&run.text);
        builder.pop_style_span();
    }
    let (mut layout, text) = builder.build();

    match case.container_property("text-indent") {
        "" | "0px" => {}
        value => {
            let mut fields = value.split(' ');
            let amount = fields
                .next()
                .and_then(px)
                .ok_or_else(|| LayoutError(format!("text-indent: {value}")))?;
            let options = IndentOptions {
                each_line: value.contains("each-line"),
                hanging: value.contains("hanging"),
            };
            layout.set_text_indent(amount, options);
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "container widths are CSS px, well within f32"
    )]
    let max_advance = (case.width + LAYOUT_UNIT) as f32;
    layout.break_all_lines(Some(max_advance));
    layout.align(alignment(case), AlignmentOptions::default());
    Ok(CaseLayout { layout, text })
}

/// Maps every byte of the processed text to the `(run, code point index)` of the
/// original character it came from.
///
/// White space processing only ever removes characters or replaces a sequence of
/// white space by a single space, and keeps runs in order, so the processed text is
/// matched against the runs' original text in lockstep: a processed character is the
/// next original character equal to it — or, for a space, the next original white
/// space character — in the current run, or failing that in a following run. Original
/// characters skipped over were removed, or collapsed into that space.
fn char_index(case: &Case, text: &str) -> Result<Vec<(usize, usize)>, LayoutError> {
    let mut table = Vec::with_capacity(text.len() + 1);
    let mut run_index = 0;
    let mut original = case.runs.first().map(|run| run.text.chars().enumerate());
    for processed in text.chars() {
        let index = loop {
            let Some(chars) = original.as_mut() else {
                return Err(LayoutError(format!(
                    "processed text {text:?} has {processed:?} beyond the runs' text"
                )));
            };
            let found = chars
                .find(|&(_, c)| c == processed || (processed == ' ' && c.is_ascii_whitespace()));
            match found {
                Some((index, _)) => break index,
                None => {
                    run_index += 1;
                    original = case
                        .runs
                        .get(run_index)
                        .map(|run| run.text.chars().enumerate());
                }
            }
        };
        table.extend(std::iter::repeat_n(
            (run_index, index),
            processed.len_utf8(),
        ));
    }
    table.push((case.runs.len(), 0));
    Ok(table)
}

/// Reads Parley's layout back as one rect per cluster, attributed to the cluster's
/// first original code point: `x` is the cluster's visual left edge, `width` its
/// advance, `y` and `height` the ascent and descent of its font around the baseline —
/// the box Chrome's `Range.getClientRects()` reports for text.
///
/// Clusters Chrome gives no (or an empty) rect are omitted: newlines, and collapsible
/// white space that hangs past the line's end edge (preserved white space keeps its
/// rect when it hangs).
pub fn parley_chars(case: &Case, laid_out: &CaseLayout) -> Result<Vec<ClusterRect>, LayoutError> {
    let CaseLayout { layout, text } = laid_out;
    let index = char_index(case, text)?;
    let mut chars = Vec::new();
    for line in layout.lines() {
        let metrics = line.metrics();
        // The extent of the line's non-hanging content.
        let line_start = f64::from(metrics.offset);
        let line_end = line_start + f64::from(metrics.advance - metrics.hanging_advance);
        for run in line.runs() {
            let font_metrics = run.font_metrics();
            let y = f64::from(metrics.baseline - font_metrics.ascent);
            let height = f64::from(font_metrics.ascent + font_metrics.descent);
            for cluster in run.clusters() {
                if cluster.is_hard_line_break() {
                    continue;
                }
                let Some(x) = cluster.visual_offset() else {
                    continue;
                };
                let x = f64::from(x);
                let advance = f64::from(cluster.advance());
                let range = cluster.text_range();
                let (run_index, char_index) = index[range.start];
                let collapsible = case.runs.get(run_index).is_some_and(|run| {
                    case.run_property(run, "white-space-collapse") != "preserve"
                });
                let hangs = text[range.clone()].chars().all(|c| c == ' ')
                    && (x >= line_end - 1e-3 || (run.is_rtl() && x + advance <= line_start + 1e-3));
                if hangs && collapsible {
                    continue;
                }
                chars.push(ClusterRect {
                    rect: CharRect {
                        run: run_index,
                        index: char_index,
                        x,
                        y,
                        width: advance,
                        height,
                    },
                    len: text[range].chars().count(),
                });
            }
        }
    }
    chars.sort_by_key(|c| (c.rect.run, c.rect.index));
    Ok(chars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StyleMap;

    fn case(runs: &[&str]) -> Case {
        Case {
            source: String::new(),
            block: 0,
            path: String::new(),
            width: 100.0,
            height: 20.0,
            lang: String::new(),
            fonts: Vec::new(),
            container: StyleMap::new(),
            styles: vec![StyleMap::new()],
            runs: runs
                .iter()
                .map(|text| Run {
                    text: (*text).into(),
                    lang: String::new(),
                    style: 0,
                })
                .collect(),
            chars: Vec::new(),
        }
    }

    #[test]
    fn char_index_traces_collapsed_text() {
        // "  a \n b" + "\t c" collapses to "a b c": leading space gone, every white
        // space sequence one space attributed to its first character.
        let case = case(&["  a \n b", "\t c"]);
        let index = char_index(&case, "a b c").unwrap();
        assert_eq!(index, vec![(0, 2), (0, 3), (0, 6), (1, 0), (1, 2), (2, 0)]);
    }

    #[test]
    fn char_index_traces_preserved_text() {
        let case = case(&["a  b", "\n", "é"]);
        let index = char_index(&case, "a  b\né").unwrap();
        assert_eq!(
            index,
            vec![
                (0, 0),
                (0, 1),
                (0, 2),
                (0, 3),
                (1, 0),
                (2, 0),
                (2, 0),
                (3, 0)
            ]
        );
    }

    #[test]
    fn char_index_rejects_text_not_in_runs() {
        let case = case(&["ab"]);
        assert!(char_index(&case, "abc").is_err());
    }
}
