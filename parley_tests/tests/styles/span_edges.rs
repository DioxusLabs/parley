// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for the edges of span boxes (`TextStyle::inline_start` / `TextStyle::inline_end`), which
//! correspond to the inline-axis margin, border and padding of a CSS inline box.
//!
//! The expectations follow the behavior of browsers.

use crate::test_name;
use crate::util::env::FONT_FAMILY_LIST;
use crate::util::{ColorBrush, TestEnv};
use parley::layout::Alignment;
use parley::style::{FontFamily, StyleProperty};
use parley::{
    AlignmentOptions, BaseDirection, InlineBox, InlineBoxKind, Layout, LineHeight,
    PositionedLayoutItem, SpanFragment, TextStyle, TreeBuilder, VerticalAlign, WhiteSpaceCollapse,
};

/// The style of a span with the given edges, which is otherwise the same as the style that
/// [`TestEnv::tree_builder`] starts with.
fn span(inline_start: f32, inline_end: f32) -> TextStyle<'static, 'static, ColorBrush> {
    TextStyle {
        font_family: FontFamily::List(FONT_FAMILY_LIST.into()),
        line_height: LineHeight::FontSizeRelative(1.0),
        inline_start,
        inline_end,
        ..TextStyle::default()
    }
}

fn build(
    env: &mut TestEnv,
    max_advance: Option<f32>,
    f: impl FnOnce(&mut TreeBuilder<'_, ColorBrush>),
) -> Layout<ColorBrush> {
    let mut builder = env.tree_builder();
    f(&mut builder);
    let (mut layout, _) = builder.build();
    layout.break_all_lines(max_advance);
    layout.align(Alignment::Start, AlignmentOptions::default());
    layout
}

/// The width of `text` on a single line.
fn width_of(env: &mut TestEnv, text: &str) -> f32 {
    build(env, None, |b| b.push_text(text)).full_width()
}

/// The `(offset, advance)` of every glyph run and inline box, per line.
fn item_extents(layout: &Layout<ColorBrush>) -> Vec<Vec<(f32, f32)>> {
    layout
        .lines()
        .map(|line| {
            line.items()
                .map(|item| match item {
                    PositionedLayoutItem::GlyphRun(run) => (run.offset(), run.advance()),
                    PositionedLayoutItem::InlineBox(inline_box) => (inline_box.x, inline_box.width),
                })
                .collect()
        })
        .collect()
}

fn fragments(layout: &Layout<ColorBrush>) -> Vec<Vec<SpanFragment>> {
    layout
        .lines()
        .map(|line| line.span_fragments().collect())
        .collect()
}

fn glyph_count(layout: &Layout<ColorBrush>) -> usize {
    layout
        .lines()
        .map(|line| {
            line.items()
                .map(|item| match item {
                    PositionedLayoutItem::GlyphRun(run) => run.glyphs().count(),
                    PositionedLayoutItem::InlineBox(_) => 0,
                })
                .sum::<usize>()
        })
        .sum()
}

/// The `(start of the text range, offset, advance)` of every glyph run on the first line.
fn runs(layout: &Layout<ColorBrush>) -> Vec<(usize, f32, f32)> {
    let line = layout.lines().next().unwrap();
    line.items()
        .filter_map(|item| match item {
            PositionedLayoutItem::GlyphRun(run) => {
                Some((run.run().text_range().start, run.offset(), run.advance()))
            }
            PositionedLayoutItem::InlineBox(_) => None,
        })
        .collect()
}

#[track_caller]
fn assert_close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.01, "{a} != {b}");
}

/// The edges are advances before and after the span's content.
#[test]
fn span_edges_offset_content() {
    let mut env = TestEnv::new(test_name!(), None);
    let (a, b, c) = (
        width_of(&mut env, "a"),
        width_of(&mut env, "b"),
        width_of(&mut env, "c"),
    );

    let layout = build(&mut env, None, |builder| {
        builder.push_text("a");
        builder.push_style_span(span(8., 4.));
        builder.push_text("b");
        builder.pop_style_span();
        builder.push_text("c");
    });

    assert_eq!(layout.len(), 1);
    let items = &item_extents(&layout)[0];
    assert_eq!(items.len(), 3);
    assert_close(items[0].0, 0.);
    assert_close(items[1].0, a + 8.);
    assert_close(items[2].0, a + 8. + b + 4.);
    assert_close(layout.full_width(), a + b + c + 12.);

    let fragments = &fragments(&layout)[0];
    assert_eq!(fragments.len(), 1);
    let fragment = fragments[0];
    assert_close(fragment.x, a);
    assert_close(fragment.advance, 8. + b + 4.);
    assert!(fragment.has_start_edge && fragment.has_end_edge && !fragment.is_rtl);
    assert_ne!(fragment.style_index, 0);
    // The content area is around the baseline.
    let metrics = *layout.lines().next().unwrap().metrics();
    assert_close(fragment.baseline, metrics.baseline);
    assert!(fragment.ascent > 0. && fragment.descent > 0.);

    let widths = layout.calculate_content_widths();
    assert_close(widths.min, a + b + c + 12.);
    assert_close(widths.max, a + b + c + 12.);
}

/// Edges set with `push_style_modification_span` generate a span box too, but they are not
/// inherited by the modification spans within it.
#[test]
fn span_edges_are_not_inherited() {
    let mut env = TestEnv::new(test_name!(), None);
    let (a, b) = (width_of(&mut env, "a"), width_of(&mut env, "b"));

    let layout = build(&mut env, None, |builder| {
        builder.push_style_modification_span(&[
            StyleProperty::InlineStart(8.),
            StyleProperty::InlineEnd(4.),
        ]);
        builder.push_text("a");
        builder.push_style_modification_span(&[StyleProperty::Underline(true)]);
        builder.push_text("b");
        builder.pop_style_span();
        builder.pop_style_span();
    });

    assert_close(layout.full_width(), 8. + a + b + 4.);
    let fragments = &fragments(&layout)[0];
    assert_eq!(fragments.len(), 1);
    assert_close(fragments[0].x, 0.);
    assert_close(fragments[0].advance, 8. + a + b + 4.);
}

/// A span without edges doesn't affect layout: in particular its text is still shaped together
/// with the text around it. It still has fragments.
#[test]
fn span_without_edges_does_not_affect_layout() {
    let mut env = TestEnv::new(test_name!(), None);
    let reference = build(&mut env, None, |b| b.push_text("of fi"));

    let layout = build(&mut env, None, |builder| {
        builder.push_text("of f");
        builder.push_style_span(span(0., 0.));
        builder.push_text("i");
        builder.pop_style_span();
    });

    // Glyph runs are split by style, but the text was shaped as a whole.
    assert_close(layout.full_width(), reference.full_width());
    assert_eq!(glyph_count(&layout), glyph_count(&reference));

    let fragments = &fragments(&layout)[0];
    assert_eq!(fragments.len(), 1);
    assert!(fragments[0].x > width_of(&mut env, "of "));
    assert!(fragments[0].advance > 0.);
    assert_close(fragments[0].x + fragments[0].advance, layout.full_width());
    assert!(fragments[0].has_start_edge && fragments[0].has_end_edge);
}

/// A span broken over several lines has its start edge on the first line only and its end edge on
/// the last line only (CSS `box-decoration-break: slice`).
#[test]
fn span_edges_when_wrapped() {
    let mut env = TestEnv::new(test_name!(), None);
    let aaa_bbb = width_of(&mut env, "aaa bbb");
    let (bbb, ccc) = (width_of(&mut env, "bbb"), width_of(&mut env, "ccc"));
    let ddd = width_of(&mut env, " ddd");
    let aaa_ = aaa_bbb - bbb;

    let layout = build(&mut env, Some(aaa_bbb + 10.), |builder| {
        builder.push_text("aaa ");
        builder.push_style_span(span(8., 4.));
        builder.push_text("bbb cccccc ccc");
        builder.pop_style_span();
        builder.push_text(" ddd");
    });

    assert_eq!(layout.len(), 3);
    let fragments = fragments(&layout);
    let [first, middle, last] = [fragments[0][0], fragments[1][0], fragments[2][0]];
    assert!(first.has_start_edge && !first.has_end_edge);
    assert_close(first.x, aaa_);
    assert!(first.advance >= 8. + bbb);

    assert!(!middle.has_start_edge && !middle.has_end_edge);
    assert_close(middle.x, 0.);

    assert!(!last.has_start_edge && last.has_end_edge);
    assert_close(last.x, 0.);
    assert_close(last.advance, ccc + 4.);
    // " ddd" comes after the end edge.
    let last_run = *item_extents(&layout)[2].last().unwrap();
    assert_close(last_run.0, ccc + 4.);
    assert_close(last_run.1, ddd);
}

/// The start edge is kept on the same line as the content following it: when that content is
/// wrapped, the line is broken before the edge.
#[test]
fn span_start_edge_stays_with_following_content() {
    let mut env = TestEnv::new(test_name!(), None);
    let aaa_ = width_of(&mut env, "aaa bbb") - width_of(&mut env, "bbb");
    let bbb = width_of(&mut env, "bbb");

    // The edge fits on the first line, but "bbb" doesn't.
    let layout = build(&mut env, Some(aaa_ + 10.), |builder| {
        builder.push_text("aaa ");
        builder.push_style_span(span(8., 0.));
        builder.push_text("bbb");
        builder.pop_style_span();
    });

    assert_eq!(layout.len(), 2);
    let fragments = fragments(&layout);
    assert!(fragments[0].is_empty());
    assert_eq!(fragments[1].len(), 1);
    assert!(fragments[1][0].has_start_edge);
    assert_close(fragments[1][0].x, 0.);
    assert_close(fragments[1][0].advance, 8. + bbb);
    assert_close(item_extents(&layout)[1][0].0, 8.);
    // The edge doesn't take up space on the first line.
    assert!(layout.lines().next().unwrap().metrics().advance <= aaa_ + 0.01);
}

/// The same is true for the start edges of nested spans, and for an inline box following them.
#[test]
fn span_start_edges_stay_with_following_inline_box() {
    let mut env = TestEnv::new(test_name!(), None);
    let aaa_ = width_of(&mut env, "aaa bbb") - width_of(&mut env, "bbb");

    let layout = build(&mut env, Some(aaa_ + 30.), |builder| {
        builder.push_text("aaa ");
        builder.push_style_span(span(8., 0.));
        builder.push_style_span(span(6., 0.));
        builder.push_inline_box(InlineBox {
            id: 0,
            kind: InlineBoxKind::InFlow,
            index: 0,
            width: 20.,
            height: 10.,
            baseline: None,
            vertical_align: VerticalAlign::BASELINE,
        });
        builder.pop_style_span();
        builder.pop_style_span();
    });

    assert_eq!(layout.len(), 2);
    let fragments = fragments(&layout);
    assert!(fragments[0].is_empty());
    assert_eq!(fragments[1].len(), 2);
    assert_close(fragments[1][0].x, 0.);
    assert_close(fragments[1][0].advance, 8. + 6. + 20.);
    assert_close(fragments[1][1].x, 8.);
    assert_close(fragments[1][1].advance, 6. + 20.);
    assert_close(item_extents(&layout)[1][0].0, 14.);
}

/// The end edge is kept on the same line as the content preceding it: when it doesn't fit, that
/// content is wrapped.
#[test]
fn span_end_edge_stays_with_preceding_content() {
    let mut env = TestEnv::new(test_name!(), None);
    let aaa_bbb = width_of(&mut env, "aaa bbb");
    let bbb = width_of(&mut env, "bbb");

    // "bbb" fits on the first line, but not together with the edge.
    let layout = build(&mut env, Some(aaa_bbb + 4.), |builder| {
        builder.push_text("aaa ");
        builder.push_style_span(span(0., 8.));
        builder.push_text("bbb");
        builder.pop_style_span();
    });

    assert_eq!(layout.len(), 2);
    let fragments = fragments(&layout);
    assert!(fragments[0].is_empty());
    assert_close(fragments[1][0].x, 0.);
    assert_close(fragments[1][0].advance, bbb + 8.);
    assert!(fragments[1][0].has_start_edge && fragments[1][0].has_end_edge);

    // With enough space, it all fits.
    let layout = build(&mut env, Some(aaa_bbb + 8.), |builder| {
        builder.push_text("aaa ");
        builder.push_style_span(span(0., 8.));
        builder.push_text("bbb");
        builder.pop_style_span();
    });
    assert_eq!(layout.len(), 1);
}

/// The start and end of a span are not soft wrap opportunities.
#[test]
fn span_edges_are_not_soft_wrap_opportunities() {
    let mut env = TestEnv::new(test_name!(), None);

    let layout = build(&mut env, Some(20.), |builder| {
        builder.push_text("aaa");
        builder.push_style_span(span(8., 4.));
        builder.push_text("bbb");
        builder.pop_style_span();
        builder.push_text("ccc");
    });
    assert_eq!(layout.len(), 1);

    let unbroken =
        width_of(&mut env, "aaa") + width_of(&mut env, "bbb") + width_of(&mut env, "ccc");
    assert_close(layout.calculate_content_widths().min, unbroken + 12.);
}

/// A span without content still takes up the space of its edges, and makes the line it is on a
/// line with content.
#[test]
fn empty_span_with_edges() {
    let mut env = TestEnv::new(test_name!(), None);
    let a = width_of(&mut env, "a");

    let layout = build(&mut env, None, |builder| {
        builder.push_text("a");
        builder.push_style_span(span(8., 4.));
        builder.pop_style_span();
        builder.push_text("b");
    });
    let items = &item_extents(&layout)[0];
    assert_close(items.last().unwrap().0, a + 12.);
    let fragments = &fragments(&layout)[0];
    assert_eq!(fragments.len(), 1);
    assert_close(fragments[0].x, a);
    assert_close(fragments[0].advance, 12.);
    assert!(fragments[0].has_start_edge && fragments[0].has_end_edge);

    // On its own.
    let layout = build(&mut env, None, |builder| {
        builder.push_style_span(span(8., 4.));
        builder.pop_style_span();
    });
    assert_eq!(layout.len(), 1);
    assert_close(layout.full_width(), 12.);
    assert!(layout.height() > 0.);
    let line_fragments: Vec<_> = layout.lines().next().unwrap().span_fragments().collect();
    assert_eq!(line_fragments.len(), 1);
    assert_close(line_fragments[0].advance, 12.);
    assert_close(layout.calculate_content_widths().min, 12.);
}

/// An empty span without edges has a zero-sized fragment where it is in the text.
#[test]
fn empty_span_without_edges() {
    let mut env = TestEnv::new(test_name!(), None);
    let a = width_of(&mut env, "a");
    let reference = build(&mut env, None, |b| b.push_text("a b"));

    let layout = build(&mut env, None, |builder| {
        builder.push_text("a");
        builder.push_style_span(span(0., 0.));
        builder.pop_style_span();
        builder.push_text(" b");
    });
    assert_close(layout.full_width(), reference.full_width());
    let fragments = &fragments(&layout)[0];
    assert_eq!(fragments.len(), 1);
    assert_close(fragments[0].x, a);
    assert_close(fragments[0].advance, 0.);
}

/// The fragment of a span includes those of the spans within it, and comes before them.
#[test]
fn nested_span_edges() {
    let mut env = TestEnv::new(test_name!(), None);
    let (a, b, c) = (
        width_of(&mut env, "a"),
        width_of(&mut env, "b"),
        width_of(&mut env, "c"),
    );

    let layout = build(&mut env, None, |builder| {
        builder.push_style_span(span(2., 3.));
        builder.push_text("a");
        builder.push_style_span(span(5., 7.));
        builder.push_text("b");
        builder.pop_style_span();
        builder.push_text("c");
        builder.pop_style_span();
    });

    let fragments = &fragments(&layout)[0];
    assert_eq!(fragments.len(), 2);
    let (outer, inner) = (fragments[0], fragments[1]);
    assert!(outer.style_index < inner.style_index);
    assert_close(outer.x, 0.);
    assert_close(outer.advance, 2. + a + 5. + b + 7. + c + 3.);
    assert_close(inner.x, 2. + a);
    assert_close(inner.advance, 5. + b + 7.);
    assert_close(layout.full_width(), outer.advance);
}

/// A span with only a nested span on a line still has a fragment on that line.
#[test]
fn nested_span_edges_when_wrapped() {
    let mut env = TestEnv::new(test_name!(), None);
    let aaa = width_of(&mut env, "aaa");

    let layout = build(&mut env, Some(aaa + 12.), |builder| {
        builder.push_style_span(span(2., 3.));
        builder.push_text("aaa ");
        builder.push_style_span(span(5., 7.));
        builder.push_text("aaa aaa");
        builder.pop_style_span();
        builder.pop_style_span();
    });

    assert_eq!(layout.len(), 3);
    let fragments = fragments(&layout);
    assert_eq!(fragments[0].len(), 1);
    assert!(fragments[0][0].has_start_edge && !fragments[0][0].has_end_edge);

    let (outer, inner) = (fragments[1][0], fragments[1][1]);
    assert!(!outer.has_start_edge && !outer.has_end_edge);
    assert!(inner.has_start_edge && !inner.has_end_edge);
    assert_close(outer.x, 0.);
    assert_close(inner.x, 0.);

    let (outer, inner) = (fragments[2][0], fragments[2][1]);
    assert!(!outer.has_start_edge && outer.has_end_edge);
    assert!(!inner.has_start_edge && inner.has_end_edge);
    assert_close(inner.advance, aaa + 7.);
    assert_close(outer.advance, aaa + 7. + 3.);
}

/// Collapsible whitespace stays on the side of the edge it was pushed on.
#[test]
fn span_edges_and_collapsed_whitespace() {
    let mut env = TestEnv::new(test_name!(), None);
    let a = width_of(&mut env, "a");
    let b = width_of(&mut env, "b");
    let a_b = width_of(&mut env, "a b");
    let space = a_b - a - b;
    let collapse = [StyleProperty::WhiteSpaceCollapse(
        WhiteSpaceCollapse::Collapse,
    )];
    let span = |inline_start, inline_end| TextStyle {
        white_space_collapse: WhiteSpaceCollapse::Collapse,
        ..span(inline_start, inline_end)
    };

    // The space is outside the span, as it starts there: it is before the start edge.
    let layout = build(&mut env, None, |builder| {
        builder.push_style_modification_span(&collapse);
        builder.push_text("a  ");
        builder.push_style_span(span(8., 4.));
        builder.push_text(" b");
        builder.pop_style_span();
        builder.push_text(" a");
    });
    let fragment = fragments(&layout)[0][0];
    assert_close(fragment.x, a + space);
    assert_close(fragment.advance, 8. + b + 4.);
    assert_close(layout.full_width(), a + space + 8. + b + 4. + space + a);

    // The spaces are inside the span.
    let layout = build(&mut env, None, |builder| {
        builder.push_style_modification_span(&collapse);
        builder.push_text("a");
        builder.push_style_span(span(8., 4.));
        builder.push_text(" b  ");
        builder.pop_style_span();
        builder.push_text(" a");
    });
    let fragment = fragments(&layout)[0][0];
    assert_close(fragment.x, a);
    assert_close(fragment.advance, 8. + space + b + space + 4.);
    assert_close(layout.full_width(), a + fragment.advance + a);
}

/// Negative edges (CSS negative margins) pull the content back.
#[test]
fn negative_span_edges() {
    let mut env = TestEnv::new(test_name!(), None);
    let (a, b) = (width_of(&mut env, "a"), width_of(&mut env, "b"));

    let layout = build(&mut env, None, |builder| {
        builder.push_text("a");
        builder.push_style_span(span(-3., -2.));
        builder.push_text("b");
        builder.pop_style_span();
        builder.push_text("a");
    });
    let items = &item_extents(&layout)[0];
    assert_close(items[1].0, a - 3.);
    assert_close(items[2].0, a - 3. + b - 2.);
    assert_close(layout.full_width(), 2. * a + b - 5.);
}

/// In right-to-left text, the start edge is on the right.
#[test]
fn span_edges_rtl() {
    let mut env = TestEnv::new(test_name!(), None);

    let layout = build(&mut env, None, |builder| {
        builder.set_base_direction(BaseDirection::Rtl);
        builder.push_text("دد ");
        builder.push_style_span(span(8., 4.));
        builder.push_text("رر");
        builder.pop_style_span();
        builder.push_text(" دد");
    });

    assert!(layout.is_rtl());
    let runs = runs(&layout);
    let run_at = |start: usize| *runs.iter().find(|run| run.0 == start).unwrap();
    // Visually: the text after the span, the end edge, the span's text, the start edge, and the
    // text before the span.
    let (before, inside, after) = (run_at(4), run_at(5), run_at(9));
    let fragment = fragments(&layout)[0][0];
    assert!(fragment.is_rtl && fragment.has_start_edge && fragment.has_end_edge);
    assert_close(fragment.x, after.1 + after.2);
    assert_close(inside.1, fragment.x + 4.);
    assert_close(fragment.advance, 4. + inside.2 + 8.);
    assert_close(before.1, fragment.x + fragment.advance);
}

/// Right-to-left text in a left-to-right paragraph keeps its order when part of it is in a span
/// with edges.
#[test]
fn span_edges_in_rtl_text_in_ltr_paragraph() {
    let mut env = TestEnv::new(test_name!(), None);

    let layout = build(&mut env, None, |builder| {
        builder.push_text("a دد ");
        builder.push_style_span(span(8., 4.));
        builder.push_text("رر");
        builder.pop_style_span();
        builder.push_text(" دد a");
    });

    assert!(!layout.is_rtl());
    let fragment = fragments(&layout)[0][0];
    assert!(fragment.is_rtl && fragment.has_start_edge && fragment.has_end_edge);

    // The right-to-left text (bytes 2 to 16) is in reverse order, as it is without the span.
    let mut rtl_runs: Vec<_> = runs(&layout)
        .into_iter()
        .filter(|run| (2..16).contains(&run.0))
        .collect();
    assert!(rtl_runs.len() >= 3);
    rtl_runs.sort_by(|a, b| b.1.total_cmp(&a.1));
    assert!(rtl_runs.is_sorted_by_key(|run| run.0));

    // The start edge is on the right of the span's text, and the end edge on the left.
    let inside = *rtl_runs.iter().find(|run| run.0 == 7).unwrap();
    assert_close(inside.1, fragment.x + 4.);
    assert_close(fragment.advance, 4. + inside.2 + 8.);
}

/// Text isn't shaped across a non-zero edge.
#[test]
fn span_edges_are_shaping_boundaries() {
    let mut env = TestEnv::new(test_name!(), None);

    let build_fi = |env: &mut TestEnv, inline_start: f32| {
        build(env, None, |builder| {
            builder.push_text("f");
            builder.push_style_span(span(inline_start, 0.));
            builder.push_text("i");
            builder.pop_style_span();
        })
    };
    let ligated = build_fi(&mut env, 0.);
    let separate = build_fi(&mut env, 8.);
    assert_eq!(glyph_count(&ligated), 1, "Roboto has an fi ligature");
    assert_eq!(glyph_count(&separate), 2);
}

/// Edges can be resized after building, e.g. for percentages of the available width.
#[test]
fn span_edges_mut() {
    let mut env = TestEnv::new(test_name!(), None);
    let b = width_of(&mut env, "b");

    let mut layout = build(&mut env, None, |builder| {
        builder.push_style_span(span(0., 0.));
        builder.push_text("a");
        builder.pop_style_span();
        builder.push_style_span(span(8., 4.));
        builder.push_text("b");
        builder.pop_style_span();
    });
    let style_index = fragments(&layout)[0][1].style_index;

    let mut edges: Vec<_> = layout.span_edges_mut().collect();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].style_index, style_index);
    assert_eq!((*edges[0].inline_start, *edges[0].inline_end), (8., 4.));
    *edges[0].inline_start = 20.;
    *edges[0].inline_end = 10.;
    drop(edges);

    layout.break_all_lines(None);
    let fragment = fragments(&layout)[0][1];
    assert_close(fragment.advance, 20. + b + 10.);
    assert_close(
        layout.calculate_content_widths().max,
        fragment.x + fragment.advance,
    );
}

/// Breaking with a line-breaking opportunity taken before a span keeps the span's fragments
/// consistent when the same layout is broken again at a different width.
#[test]
fn span_edges_rebreak() {
    let mut env = TestEnv::new(test_name!(), None);

    let mut layout = build(&mut env, Some(40.), |builder| {
        builder.push_text("aaa ");
        builder.push_style_span(span(8., 4.));
        builder.push_text("bbb ccc");
        builder.pop_style_span();
        builder.push_text(" ddd");
    });
    let narrow = fragments(&layout);
    assert!(layout.len() > 1);

    layout.break_all_lines(None);
    assert_eq!(layout.len(), 1);
    assert_eq!(fragments(&layout)[0].len(), 1);

    layout.break_all_lines(Some(40.));
    assert_eq!(fragments(&layout), narrow);
}
