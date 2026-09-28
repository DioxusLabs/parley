// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Text wrapping tests.

use crate::test_name;
use crate::util::{ColorBrush, TestEnv};
use parley::{
    Alignment, AlignmentOptions, BreakReason, CHROMIUM_LINE_BREAK_OVERRIDE, InlineBox,
    InlineBoxKind, OverflowWrap, StyleProperty, TextWrapMode, TreeBuilder, VerticalAlign,
    WordBreak,
};
use peniko::color::palette::css;

fn test_wrap(
    env: &mut TestEnv,
    pattern: Option<&str>,
    wrap_property: StyleProperty<'_, ColorBrush>,
    color: ColorBrush,
    wrap_width: f32,
) {
    test_wrap_with_custom_text(
        env,
        "Most words are short. But Antidisestablishmentarianism is long and needs to wrap.",
        pattern,
        wrap_property,
        color,
        wrap_width,
    );
}

fn test_wrap_with_custom_text(
    env: &mut TestEnv,
    text: &str,
    pattern: Option<&str>,
    wrap_property: StyleProperty<'_, ColorBrush>,
    color: ColorBrush,
    wrap_width: f32,
) {
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::Brush(ColorBrush::new(css::RED)));

    if let Some(pattern) = pattern {
        let start = text.find(pattern).unwrap();
        let range = start..start + pattern.len();
        builder.push(StyleProperty::Brush(color), range.clone());
        builder.push(StyleProperty::Underline(true), range.clone());
        builder.push(wrap_property, range.clone());
    }

    let mut layout = builder.build(text);
    layout.break_all_lines(Some(wrap_width));
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

#[test]
fn overflow_wrap_off() {
    let mut env = TestEnv::new(test_name!(), None);

    test_wrap(
        &mut env,
        None,
        StyleProperty::OverflowWrap(OverflowWrap::default()),
        ColorBrush::default(),
        120.0,
    );
}

#[test]
fn overflow_wrap_first_half() {
    let mut env = TestEnv::new(test_name!(), None);

    test_wrap(
        &mut env,
        Some("Antidis"),
        StyleProperty::OverflowWrap(OverflowWrap::Anywhere),
        ColorBrush::new(css::BLUE),
        120.0,
    );
}

#[test]
fn overflow_wrap_second_half() {
    let mut env = TestEnv::new(test_name!(), None);

    test_wrap(
        &mut env,
        Some("anism"),
        StyleProperty::OverflowWrap(OverflowWrap::Anywhere),
        ColorBrush::new(css::BLUE),
        120.0,
    );
}

#[test]
fn overflow_wrap_during() {
    let mut env = TestEnv::new(test_name!(), None);

    test_wrap(
        &mut env,
        Some("establishment"),
        StyleProperty::OverflowWrap(OverflowWrap::Anywhere),
        ColorBrush::new(css::BLUE),
        120.0,
    );
}

#[test]
fn overflow_wrap_everywhere() {
    let mut env = TestEnv::new(test_name!(), None);

    test_wrap(
        &mut env,
        Some("Most words are short. But Antidisestablishmentarianism is long and needs to wrap."),
        StyleProperty::OverflowWrap(OverflowWrap::Anywhere),
        ColorBrush::new(css::BLUE),
        120.0,
    );
}

#[test]
fn overflow_wrap_narrow() {
    let mut env = TestEnv::new(test_name!(), None);

    test_wrap(
        &mut env,
        Some("Most words are short. But Antidisestablishmentarianism is long and needs to wrap."),
        StyleProperty::OverflowWrap(OverflowWrap::Anywhere),
        ColorBrush::new(css::BLUE),
        5.0,
    );
}

#[test]
fn overflow_wrap_anywhere_min_content_width() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "Hello world!\nLonger line with a looooooooong word.";
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::OverflowWrap(OverflowWrap::Anywhere));

    let mut layout = builder.build(text);

    layout.break_all_lines(Some(layout.calculate_content_widths().min));
    layout.align(Alignment::Start, AlignmentOptions::default());
    env.check_layout_snapshot(&layout);
}

#[test]
fn overflow_wrap_break_word_min_content_width() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "Hello world!\nLonger line with a looooooooong word.";
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::OverflowWrap(OverflowWrap::BreakWord));

    let mut layout = builder.build(text);

    layout.break_all_lines(Some(layout.calculate_content_widths().min));
    layout.align(Alignment::Start, AlignmentOptions::default());
    env.check_layout_snapshot(&layout);
}

#[test]
fn word_break_break_all_first_half() {
    let mut env = TestEnv::new(test_name!(), None);

    test_wrap(
        &mut env,
        Some("Antidis"),
        StyleProperty::WordBreak(WordBreak::BreakAll),
        ColorBrush::new(css::GREEN),
        120.0,
    );
}

#[test]
fn word_break_break_all_second_half() {
    let mut env = TestEnv::new(test_name!(), None);

    test_wrap(
        &mut env,
        Some("anism"),
        StyleProperty::WordBreak(WordBreak::BreakAll),
        ColorBrush::new(css::GREEN),
        120.0,
    );
}

#[test]
fn word_break_break_all_during() {
    let mut env = TestEnv::new(test_name!(), None);

    test_wrap(
        &mut env,
        Some("establishment"),
        StyleProperty::WordBreak(WordBreak::BreakAll),
        ColorBrush::new(css::GREEN),
        120.0,
    );
}

#[test]
fn word_break_break_all_everywhere() {
    let mut env = TestEnv::new(test_name!(), None);

    test_wrap(
        &mut env,
        Some("Most words are short. But Antidisestablishmentarianism is long and needs to wrap."),
        StyleProperty::WordBreak(WordBreak::BreakAll),
        ColorBrush::new(css::GREEN),
        120.0,
    );
}

#[test]
fn word_break_break_all_min_content_width() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "Hello world!\nLonger line with a looooooooong word.";
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::WordBreak(WordBreak::BreakAll));

    let mut layout = builder.build(text);

    layout.break_all_lines(Some(layout.calculate_content_widths().min));
    layout.align(Alignment::Start, AlignmentOptions::default());
    // This snapshot will have slightly different line wrapping than the corresponding overflow-wrap test. This is to be
    // expected and matches browser/CSS behavior.
    env.check_layout_snapshot(&layout);
}

#[test]
fn word_break_wpt007() {
    // See http://wpt.live/css/css-text/word-break/word-break-break-all-inline-007.tentative.html
    //
    // All browsers fail this currently, but we pass it. This means that word_break_break_all_first_half doesn't match
    // what any browsers do currently, but should be theoretically correct.
    let mut env = TestEnv::new(test_name!(), None);

    test_wrap_with_custom_text(
        &mut env,
        "aaaaaaabbbbbbbcccccc",
        Some("bbbbbbb"),
        StyleProperty::WordBreak(WordBreak::BreakAll),
        ColorBrush::new(css::GREEN),
        55.0,
    );
}

#[test]
fn word_break_keep_all() {
    let mut env = TestEnv::new(test_name!(), None);

    let mut test_text = |text, name, wrap_width| {
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::WordBreak(WordBreak::KeepAll));

        let mut layout = builder.build(text);

        layout.break_all_lines(Some(wrap_width));
        layout.align(Alignment::Start, AlignmentOptions::default());
        env.with_name(name).check_layout_snapshot(&layout);
    };

    // These are the word-break-keep-all tests from WPT:
    // https://wpt.fyi/results/css/css-text/word-break?label=experimental&label=master&aligned
    test_text("Latin latin latin latin", "latin", 120.0);
    // These will all show up as boxes because CJK fonts are quite large (several megabytes per language) and could
    // bloat the repository. Line break analysis should work the same regardless of font, however.
    test_text("日本語 日本語 日本語", "japanese", 60.0);
    // Jamo decomposed on purpose
    test_text("한글이 한글이 한글이", "korean", 60.0);
    // TODO: we fail this test; so does Safari
    // https://wpt.fyi/results/css/css-text/word-break/word-break-keep-all-003.html
    // test_text("และ และและ", "thai", 65.0);
    test_text("フォ フォ", "ID_and_CJ", 30.0);
    // Jamo decomposed on purpose
    test_text("애기판다 애기판다", "korean_hangul_jamos", 90.0);
}

#[test]
fn text_wrap_mode_nowrap_disables_soft_wraps() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "Most words are short. But Antidisestablishmentarianism is long and needs to wrap.";
    let wrap_width = 120.0;

    let mut baseline_layout = env.ranged_builder(text).build(text);
    baseline_layout.break_all_lines(Some(wrap_width));
    assert!(
        baseline_layout.len() > 1,
        "Expected baseline layout to wrap with width {wrap_width}"
    );

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::TextWrapMode(TextWrapMode::NoWrap));
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(wrap_width));

    assert_eq!(
        layout.len(),
        1,
        "Applying TextWrapMode::NoWrap should prevent soft wrapping"
    );

    let line_advance = layout
        .lines()
        .next()
        .expect("layout should have one line")
        .metrics()
        .advance;
    assert!(
        line_advance > wrap_width,
        "Line advance {line_advance} should overflow the requested width {wrap_width}"
    );
}

#[test]
fn text_wrap_mode_allows_break_before_nowrap_span() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "Hello world!";
    let prefix = "Hello ";

    let prefix_width = {
        let mut layout = env.ranged_builder(prefix).build(prefix);
        layout.break_all_lines(None);
        layout.width()
    };

    let wrap_width = prefix_width + 1.0;

    let start = text.find("world!").unwrap();
    let mut builder = env.ranged_builder(text);
    builder.push(
        StyleProperty::TextWrapMode(TextWrapMode::NoWrap),
        // `start..text.len()` === "world!"
        start..text.len(),
    );

    let mut layout = builder.build(text);
    layout.break_all_lines(Some(wrap_width));

    assert_eq!(
        layout.len(),
        2,
        "Layout should still wrap before the NoWrap span boundary"
    );

    let first_line = layout.get(0).unwrap();
    let second_line = layout.get(1).unwrap();
    assert_eq!(
        &text[first_line.text_range()],
        "Hello ",
        "First line should end before the NoWrap span"
    );
    assert_eq!(
        &text[second_line.text_range()],
        "world!",
        "Second line should contain the NoWrap span"
    );
}

#[test]
fn text_wrap_mode_updates_min_content_width() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "Tags: London UK Paris FR";
    let span_text = "London UK";
    let span_start = text.find(span_text).unwrap();
    let span_range = span_start..span_start + span_text.len();

    let widths_wrap = {
        let layout = env.ranged_builder(text).build(text);
        layout.calculate_content_widths()
    };

    let widths_nowrap = {
        let mut builder = env.ranged_builder(text);
        builder.push(
            StyleProperty::TextWrapMode(TextWrapMode::NoWrap),
            span_range.clone(),
        );
        let layout = builder.build(text);
        layout.calculate_content_widths()
    };

    let span_width = {
        let mut layout = env.ranged_builder(span_text).build(span_text);
        layout.break_all_lines(None);
        layout.width()
    };

    assert!(
        widths_wrap.min < span_width,
        "Without NoWrap, min content width {} should be smaller than the span width {}",
        widths_wrap.min,
        span_width
    );
    assert!(
        widths_nowrap.min >= span_width - 0.5,
        "With NoWrap, min content width {} should be at least the span width {}",
        widths_nowrap.min,
        span_width
    );
}

#[test]
fn wrap_url_breaks_after_slash() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "https://www.example.com/path/to/resource";

    let mut layout = env.ranged_builder(text).build(text);
    layout.break_all_lines(Some(150.0));
    layout.align(Alignment::Start, AlignmentOptions::default());

    assert!(layout.len() > 1);

    let last_line_index = layout.len() - 1;
    for (i, line) in layout.lines().enumerate() {
        if i == last_line_index {
            break;
        }

        assert_eq!(line.break_reason(), BreakReason::Regular,);

        let line_text = &text[line.text_range()];
        assert!(line_text.ends_with('/'),);
    }

    env.check_layout_snapshot(&layout);
}

#[test]
fn wrap_url_override_no_break_after_slash() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "https://www.example.com/path/to/resource";

    let wrap_width = 150.0;

    let mut builder = env.ranged_builder(text);
    builder.set_line_break_override(Some(&CHROMIUM_LINE_BREAK_OVERRIDE));
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(wrap_width));
    layout.align(Alignment::Start, AlignmentOptions::default());

    assert_eq!(layout.len(), 1);

    let line_advance = layout
        .lines()
        .next()
        .expect("layout should have one line")
        .metrics()
        .advance;
    assert!(line_advance > wrap_width);

    env.check_layout_snapshot(&layout);
}

#[test]
fn line_break_override_does_not_affect_forced_breaks() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "a\nb";

    for forced in [false, true] {
        let line_break_override = move |_| Some(forced);
        let mut builder = env.ranged_builder(text);
        builder.set_line_break_override(Some(&line_break_override));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);

        let lines: Vec<_> = layout
            .lines()
            .map(|line| &text[line.text_range()])
            .collect();
        assert_eq!(lines, ["a\n", "b"], "override returning Some({forced})");
    }
}

fn push_wrap_mode_span(
    builder: &mut TreeBuilder<'_, ColorBrush>,
    mode: TextWrapMode,
    contents: impl FnOnce(&mut TreeBuilder<'_, ColorBrush>),
) {
    builder.push_style_modification_span(&[StyleProperty::TextWrapMode(mode)]);
    contents(builder);
    builder.pop_style_span();
}

fn nowrap(
    builder: &mut TreeBuilder<'_, ColorBrush>,
    contents: impl FnOnce(&mut TreeBuilder<'_, ColorBrush>),
) {
    push_wrap_mode_span(builder, TextWrapMode::NoWrap, contents);
}

fn wrap(
    builder: &mut TreeBuilder<'_, ColorBrush>,
    contents: impl FnOnce(&mut TreeBuilder<'_, ColorBrush>),
) {
    push_wrap_mode_span(builder, TextWrapMode::Wrap, contents);
}

fn push_box(builder: &mut TreeBuilder<'_, ColorBrush>) {
    builder.push_inline_box(InlineBox {
        id: 0,
        kind: InlineBoxKind::InFlow,
        index: 0,
        width: 10.0,
        height: 10.0,
        baseline: None,
        vertical_align: VerticalAlign::BASELINE,
    });
}

/// Returns the number of lines when breaking at every soft wrap opportunity, and whether the
/// min-content width is smaller than the max-content width.
fn element_boundary_breaks(
    contents: impl FnOnce(&mut TreeBuilder<'_, ColorBrush>),
) -> (usize, bool) {
    let mut env = TestEnv::new(test_name!(), None);
    let mut builder = env.tree_builder();
    contents(&mut builder);
    let (mut layout, _) = builder.build();
    let widths = layout.calculate_content_widths();
    layout.break_all_lines(Some(0.0));
    (layout.len(), widths.min < widths.max)
}

#[test]
fn element_boundary_wrap_follows_nearest_common_ancestor() {
    // `<nowrap>口</nowrap>口`: the boundary is controlled by the (wrapping) root.
    let breaks = element_boundary_breaks(|b| {
        nowrap(b, |b| b.push_text("口"));
        b.push_text("口");
    });
    assert_eq!(breaks, (2, true));

    // `<nowrap>口</nowrap><nowrap>口</nowrap>`
    let breaks = element_boundary_breaks(|b| {
        nowrap(b, |b| b.push_text("口"));
        nowrap(b, |b| b.push_text("口"));
    });
    assert_eq!(breaks, (2, true));

    // `口<nowrap>口</nowrap>`
    let breaks = element_boundary_breaks(|b| {
        b.push_text("口");
        nowrap(b, |b| b.push_text("口"));
    });
    assert_eq!(breaks, (2, true));

    // `<nowrap>口口</nowrap>`
    let breaks = element_boundary_breaks(|b| nowrap(b, |b| b.push_text("口口")));
    assert_eq!(breaks, (1, false));

    // `<nowrap><wrap>口</wrap><wrap>口</wrap></nowrap>`: the boundary is controlled by the
    // non-wrapping common ancestor.
    let breaks = element_boundary_breaks(|b| {
        nowrap(b, |b| {
            wrap(b, |b| b.push_text("口"));
            wrap(b, |b| b.push_text("口"));
        });
    });
    assert_eq!(breaks, (1, false));
}

#[test]
fn element_boundary_wrap_after_space_follows_space() {
    // `<nowrap><wrap>口 </wrap><wrap>口</wrap></nowrap>`: the opportunity after a space is
    // controlled by the space's own style.
    let breaks = element_boundary_breaks(|b| {
        nowrap(b, |b| {
            wrap(b, |b| b.push_text("口 "));
            wrap(b, |b| b.push_text("口"));
        });
    });
    assert_eq!(breaks, (2, true));

    // `<nowrap>口 </nowrap>口`
    let breaks = element_boundary_breaks(|b| {
        nowrap(b, |b| b.push_text("口 "));
        b.push_text("口");
    });
    assert_eq!(breaks.0, 1);
}

#[test]
fn inline_box_wrap_follows_nearest_common_ancestor() {
    // `<nowrap>口</nowrap><nowrap>[box]口</nowrap>`
    let breaks = element_boundary_breaks(|b| {
        nowrap(b, |b| b.push_text("口"));
        nowrap(b, |b| {
            push_box(b);
            b.push_text("口");
        });
    });
    assert_eq!(breaks, (2, true));

    // `<nowrap>[box]</nowrap>口`
    let breaks = element_boundary_breaks(|b| {
        nowrap(b, push_box);
        b.push_text("口");
    });
    assert_eq!(breaks, (2, true));

    // `<nowrap>[box]</nowrap><nowrap>[box]口</nowrap>`
    let breaks = element_boundary_breaks(|b| {
        nowrap(b, push_box);
        nowrap(b, |b| {
            push_box(b);
            b.push_text("口");
        });
    });
    assert_eq!(breaks, (2, true));

    // `<nowrap>口[box]口</nowrap>`
    let breaks = element_boundary_breaks(|b| {
        nowrap(b, |b| {
            b.push_text("口");
            push_box(b);
            b.push_text("口");
        });
    });
    assert_eq!(breaks, (1, false));

    // `<nowrap><wrap>[box]</wrap><wrap>[box]</wrap>口</nowrap>`
    let breaks = element_boundary_breaks(|b| {
        nowrap(b, |b| {
            wrap(b, push_box);
            wrap(b, push_box);
            b.push_text("口");
        });
    });
    assert_eq!(breaks, (1, false));
}
