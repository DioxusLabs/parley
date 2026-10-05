// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;

use super::utils::{
    ColorBrush,
    fonts::{FONT_FAMILY_LIST, create_font_context},
};
use crate::{
    Alignment, AlignmentOptions, FontFamily, IndentOptions, InlineBox, InlineBoxKind, Layout,
    LayoutContext, PositionedLayoutItem, StyleProperty, VerticalAlign,
};

#[test]
fn clear_resets_to_new() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();

    static TEXT: &str = "Some text that will wrap across several lines.";

    let mut builder = lcx.ranged_builder(&mut fcx, TEXT, 1., false);
    builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
    let mut layout = builder.build(TEXT);
    layout.set_text_indent(10., IndentOptions::default());
    layout.break_all_lines(Some(50.));
    layout.align(Alignment::Center, AlignmentOptions::default());
    assert!(layout.lines().len() > 1);
    assert!(layout.width() > 0.);
    assert!(layout.height() > 0.);

    layout.clear();

    assert_eq!(layout.data, Layout::new().data);
}

fn assert_positioned_boxes(layout: &Layout<ColorBrush>) {
    let expected: Vec<_> = layout
        .lines()
        .enumerate()
        .flat_map(|(line_index, line)| {
            line.items().filter_map(move |item| match item {
                PositionedLayoutItem::InlineBox(inline_box) => Some((line_index, inline_box)),
                _ => None,
            })
        })
        .collect();
    let actual: Vec<_> = layout.positioned_inline_boxes().collect();
    assert_eq!(actual.len(), expected.len());
    for (actual, (line_index, expected)) in actual.iter().zip(expected) {
        assert_eq!(actual.id, expected.id);
        assert_eq!(actual.kind, expected.kind);
        assert_eq!(actual.line_index, line_index);
        assert!(
            (actual.x - expected.x).abs() < 0.001,
            "{actual:?} {expected:?}"
        );
        assert_eq!(actual.y, expected.y);
        assert_eq!(actual.width, expected.width);
        assert_eq!(actual.height, expected.height);
        assert_eq!(actual.baseline, expected.baseline);
    }
}

#[test]
fn positioned_boxes_follow_breaking_alignment_and_relayout() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    let text = "Hello office world! مرحبا بالعالم שלום עולם goodbye world!";
    let mut builder = lcx.ranged_builder(&mut fcx, text, 1., false);
    builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
    builder.push_default(StyleProperty::LetterSpacing(0.7));
    builder.push_default(StyleProperty::WordSpacing(1.3));
    for (id, index, kind, vertical_align) in [
        (1, 6, InlineBoxKind::InFlow, VerticalAlign::BASELINE),
        (2, 13, InlineBoxKind::OutOfFlow, VerticalAlign::BASELINE),
        (3, 26, InlineBoxKind::InFlow, VerticalAlign::TOP),
        (4, text.len(), InlineBoxKind::InFlow, VerticalAlign::BOTTOM),
    ] {
        assert!(text.is_char_boundary(index));
        builder.push_inline_box(InlineBox {
            id,
            index,
            kind,
            vertical_align,
            width: 20.,
            height: 30.,
            baseline: Some(15.),
        });
    }
    let mut layout = builder.build(text);
    assert_eq!(layout.positioned_inline_boxes().count(), 0);
    layout.set_text_indent(12., IndentOptions::default());
    for width in [130., 500., 90.] {
        layout.break_all_lines(Some(width));
        assert_positioned_boxes(&layout);
        for alignment in [
            Alignment::Start,
            Alignment::Center,
            Alignment::Right,
            Alignment::Justify,
        ] {
            layout.align(alignment, AlignmentOptions::default());
            assert_positioned_boxes(&layout);
        }
    }
    let mut cloned = layout.clone();
    assert_positioned_boxes(&cloned);
    cloned.clear();
    assert_eq!(cloned.positioned_inline_boxes().count(), 0);
}

#[test]
fn positioned_boxes_preserve_rtl_visual_order_and_custom_boxes() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    let text = "שלום עולם مرحبا שלום";
    let mut builder = lcx.ranged_builder(&mut fcx, text, 1., true);
    builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
    for (id, index, kind) in [
        (1, 0, InlineBoxKind::InFlow),
        (2, 9, InlineBoxKind::CustomOutOfFlow),
        (3, text.len(), InlineBoxKind::OutOfFlow),
    ] {
        assert!(text.is_char_boundary(index));
        builder.push_inline_box(InlineBox {
            id,
            index,
            kind,
            vertical_align: VerticalAlign::BASELINE,
            width: 15.,
            height: 25.,
            baseline: None,
        });
    }
    let mut layout = builder.build(text);
    for width in [80., 400.] {
        layout.break_all_lines(Some(width));
        layout.align(Alignment::Justify, AlignmentOptions::default());
        assert_positioned_boxes(&layout);
    }
}

#[test]
fn positioned_boxes_follow_controls_and_preserved_whitespace() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    for text in [
        "a\u{200b}b\u{200d}c\u{2060}d ",
        "a\tb\nc ",
        "office ffi fi ",
    ] {
        let mut builder = lcx.ranged_builder(&mut fcx, text, 1., false);
        builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
        builder.push_default(StyleProperty::LetterSpacing(2.));
        builder.push_inline_box(InlineBox {
            id: 1,
            index: text.len(),
            kind: InlineBoxKind::InFlow,
            vertical_align: VerticalAlign::BASELINE,
            width: 20.,
            height: 20.,
            baseline: None,
        });
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(500.));
        assert_positioned_boxes(&layout);
        layout.align(Alignment::Justify, AlignmentOptions::default());
        assert_positioned_boxes(&layout);
    }
}
