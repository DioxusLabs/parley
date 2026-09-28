// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;
use core::ops::Range;

use crate::resolve::{ResolvedStyle, StyleRun};
use crate::{Brush, InlineBoxKind, LayoutContext, TextWrapMode, WhiteSpaceCollapse};

use parley_engine::break_overrides::LineBreakOverrideFn;

use parley_engine::{AnalysisOptions, LineBreakConfig};

use parlance::BaseDirection;

pub(crate) fn analyze_text<B: Brush>(
    lcx: &mut LayoutContext<B>,
    text: &str,
    base_direction: BaseDirection,
    line_break_override: Option<&LineBreakOverrideFn>,
) {
    let text = if text.is_empty() { " " } else { text };

    // Collect the style runs' line break configurations. Gaps use the default configuration, so
    // only non-default configurations need an entry, and adjacent equal configurations are merged.
    //
    // Separately, collect the `break-spaces` runs, which allow wrapping after each preserved space
    // or tab. Adjacent runs are merged, so that an opportunity is not created before the first
    // space of a sequence spanning a style boundary.
    lcx.line_break.clear();
    lcx.break_spaces.clear();
    for style_run in lcx.style_runs.iter() {
        let style = &lcx.style_table[style_run.style_index as usize];
        let line_break = LineBreakConfig {
            word_break: style.word_break,
            line_break: style.line_break,
            language: style.locale,
        };
        if line_break != LineBreakConfig::default() {
            match lcx.line_break.last_mut() {
                Some((range, last))
                    if range.end == style_run.range.start && *last == line_break =>
                {
                    range.end = style_run.range.end;
                }
                _ => lcx.line_break.push((style_run.range.clone(), line_break)),
            }
        }
        if style.white_space_collapse == WhiteSpaceCollapse::BreakSpaces {
            match lcx.break_spaces.last_mut() {
                Some(last) if last.end == style_run.range.start => last.end = style_run.range.end,
                _ => lcx.break_spaces.push(style_run.range.clone()),
            }
        }
    }

    resolve_no_wrap_ranges(lcx, text);

    let options = AnalysisOptions {
        base_direction,
        line_break: &lcx.line_break,
        break_spaces: &lcx.break_spaces,
        no_wrap: &lcx.no_wrap,
        line_break_override,
    };
    lcx.analyzer.analyze(text, &options, &mut lcx.analysis);
}

/// Collects the ranges of `text` within which soft wrap opportunities are suppressed by
/// `text-wrap-mode: nowrap`.
///
/// Following CSS Text 3 § 5.1, the `text-wrap-mode` controlling an opportunity between two
/// characters is that of their nearest common ancestor, except that the style of a space controls
/// the opportunity after it. Within a style run, that's the style of the run itself. At style run
/// boundaries, a range spanning the characters on either side of the boundary suppresses it.
fn resolve_no_wrap_ranges<B: Brush>(lcx: &mut LayoutContext<B>, text: &str) {
    lcx.no_wrap.clear();
    let styles = &lcx.style_table;
    if !has_no_wrap(styles) {
        return;
    }

    let mut prev_run: Option<&StyleRun> = None;
    for style_run in lcx.style_runs.iter().filter(|run| !run.range.is_empty()) {
        let pos = style_run.range.start;
        if let Some(prev_run) = prev_run
            && pos == prev_run.range.end
            && let Some(before) = text[..pos].chars().next_back()
            && let Some(after) = text[pos..].chars().next()
            && wrap_mode_between(
                styles,
                prev_run.style_index,
                is_wrapping_space(before),
                style_run.style_index,
            ) == TextWrapMode::NoWrap
        {
            push_no_wrap_range(
                &mut lcx.no_wrap,
                pos - before.len_utf8()..pos + after.len_utf8(),
            );
        }
        if styles[usize::from(style_run.style_index)].text_wrap_mode == TextWrapMode::NoWrap {
            push_no_wrap_range(&mut lcx.no_wrap, style_run.range.clone());
        }
        prev_run = Some(style_run);
    }

    // The soft wrap opportunity before a character following inline boxes is the one after the
    // last in-flow box, see `resolve_inline_box_breaks`.
    let no_wrap_len = lcx.no_wrap.len();
    let mut boxes = lcx
        .inline_boxes
        .iter()
        .filter(|inline_box| inline_box.inline_box.kind == InlineBoxKind::InFlow)
        .peekable();
    while let Some(inline_box) = boxes.next() {
        let pos = inline_box.inline_box.index;
        if inline_box.break_after
            || boxes
                .peek()
                .is_some_and(|next| next.inline_box.index == pos)
        {
            continue;
        }
        if let Some(before) = text[..pos].chars().next_back()
            && let Some(after) = text[pos..].chars().next()
        {
            lcx.no_wrap
                .push(pos - before.len_utf8()..pos + after.len_utf8());
        }
    }
    if lcx.no_wrap.len() > no_wrap_len {
        merge_no_wrap_ranges(&mut lcx.no_wrap);
    }
}

/// Sorts `ranges` and merges overlapping ranges.
fn merge_no_wrap_ranges(ranges: &mut Vec<Range<usize>>) {
    ranges.sort_unstable_by_key(|range| range.start);
    let mut merged = 0;
    for idx in 0..ranges.len() {
        let range = ranges[idx].clone();
        if merged > 0 && ranges[merged - 1].end > range.start {
            ranges[merged - 1].end = ranges[merged - 1].end.max(range.end);
        } else {
            ranges[merged] = range;
            merged += 1;
        }
    }
    ranges.truncate(merged);
}

/// Appends `range` to `ranges`, merging it into the last range if they overlap.
///
/// Adjacent ranges are not merged, as the opportunity at their shared boundary is not suppressed.
fn push_no_wrap_range(ranges: &mut Vec<Range<usize>>, range: Range<usize>) {
    match ranges.last_mut() {
        Some(last) if last.end > range.start => last.end = last.end.max(range.end),
        _ => ranges.push(range),
    }
}

/// Resolves whether there are soft wrap opportunities before and after each in-flow inline box,
/// following the same rules as [`resolve_no_wrap_ranges`] with the box taking the style of the
/// span containing it.
///
/// The inline boxes must be sorted by text index. This must run before [`analyze_text`], which
/// accounts for the resolved breaks.
pub(crate) fn resolve_inline_box_breaks<B: Brush>(lcx: &mut LayoutContext<B>, text: &str) {
    let styles = &lcx.style_table;
    let style_runs = &lcx.style_runs;
    let boxes = &mut lcx.inline_boxes;
    if !has_no_wrap(styles) {
        for inline_box in boxes.iter_mut() {
            inline_box.break_before = true;
            inline_box.break_after = true;
        }
        return;
    }

    let style_index_at = |pos: usize| {
        let run_idx = style_runs.partition_point(|run| run.range.end <= pos);
        style_runs.get(run_idx).map_or(0, |run| run.style_index)
    };
    let is_in_flow = |inline_box: &&crate::inline_box::LayoutInlineBox| {
        inline_box.inline_box.kind == InlineBoxKind::InFlow
    };

    for idx in 0..boxes.len() {
        let pos = boxes[idx].inline_box.index;
        let style_index = boxes[idx].parent_style_index;

        let before = boxes[..idx]
            .iter()
            .rev()
            .take_while(|prev| prev.inline_box.index == pos)
            .find(is_in_flow)
            .map(|prev| (prev.parent_style_index, false))
            .or_else(|| {
                let before = text[..pos].chars().next_back()?;
                Some((
                    style_index_at(pos - before.len_utf8()),
                    is_wrapping_space(before),
                ))
            });
        let after = boxes[idx + 1..]
            .iter()
            .take_while(|next| next.inline_box.index == pos)
            .find(is_in_flow)
            .map(|next| next.parent_style_index)
            .or_else(|| (pos < text.len()).then(|| style_index_at(pos)));

        let break_before = before.is_none_or(|(before, before_is_space)| {
            wrap_mode_between(styles, before, before_is_space, style_index) == TextWrapMode::Wrap
        });
        let break_after = after.is_none_or(|after| {
            wrap_mode_between(styles, style_index, false, after) == TextWrapMode::Wrap
        });
        boxes[idx].break_before = break_before;
        boxes[idx].break_after = break_after;
    }
}

fn has_no_wrap<B: Brush>(styles: &[ResolvedStyle<B>]) -> bool {
    styles
        .iter()
        .any(|style| style.text_wrap_mode == TextWrapMode::NoWrap)
}

/// Whether the soft wrap opportunity after `c` is created by `c` itself, i.e., whether `c` is a
/// breaking space.
fn is_wrapping_space(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t'
            | '\u{1680}'
            | '\u{2000}'..='\u{2006}'
            | '\u{2008}'..='\u{200A}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

/// The `text-wrap-mode` controlling a soft wrap opportunity between content with style `before`
/// and following content with style `after`.
///
/// If `before_is_space`, that's the mode of `before`; otherwise, it's the mode of the nearest
/// common ancestor of the two styles.
fn wrap_mode_between<B: Brush>(
    styles: &[ResolvedStyle<B>],
    mut before: u16,
    before_is_space: bool,
    mut after: u16,
) -> TextWrapMode {
    if before_is_space {
        after = before;
    }
    // The style table is ordered parent-first, so the style with the larger index is never an
    // ancestor of the other.
    while before != after {
        if before > after {
            before = styles[usize::from(before)].parent;
        } else {
            after = styles[usize::from(after)].parent;
        }
    }
    styles[usize::from(before)].text_wrap_mode
}
