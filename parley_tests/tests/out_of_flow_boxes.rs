// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{
    test_name,
    util::{ColorBrush, TestEnv},
};
use parley::{FloatedBox, Layout, PositionedLayoutItem};

#[test]
fn out_of_flow_box_has_no_effect_on_layout() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "Hello world! This is a test of out-of-flow boxes.";

    // Reference layout without any boxes
    let builder_ref = env.ranged_builder(text);
    let layout_ref = builder_ref.build(text);
    let widths_ref = layout_ref.calculate_content_widths();

    // Treatment layout with a floated box
    let mut builder_oof = env.ranged_builder(text);
    builder_oof.push_floated_box(FloatedBox { id: 42, index: 6 });
    let layout_oof = builder_oof.build(text);
    let widths_oof = layout_oof.calculate_content_widths();

    // Content widths should be unaffected
    assert_eq!(widths_ref.min, widths_oof.min);
    assert_eq!(widths_ref.max, widths_oof.max);

    // Perform line breaking and alignment
    let mut layout_ref = layout_ref;
    layout_ref.break_all_lines(Some(100.0));

    let mut layout_oof = layout_oof;
    layout_oof.break_all_lines(Some(100.0));

    // Layout dimensions and line count should be unaffected
    assert_eq!(layout_ref.width(), layout_oof.width());
    assert_eq!(layout_ref.height(), layout_oof.height());
    assert_eq!(layout_ref.len(), layout_oof.len());

    // Glyph positions should be identical
    let glyph_xs = |layout: &Layout<ColorBrush>| -> Vec<f32> {
        layout
            .lines()
            .flat_map(|line| {
                line.items().filter_map(|item| match item {
                    PositionedLayoutItem::GlyphRun(run) => {
                        Some(run.positioned_glyphs().map(|g| g.x).collect::<Vec<_>>())
                    }
                    _ => None,
                })
            })
            .flatten()
            .collect()
    };
    assert_eq!(glyph_xs(&layout_ref), glyph_xs(&layout_oof),);
}

/// An out-of-flow box is not in-flow content, so it must not introduce a line break
/// opportunity: a word containing one still overflows rather than being split.
#[test]
fn out_of_flow_box_is_not_a_line_break_opportunity() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "unbreakable";

    let line_ranges = |layout: &Layout<ColorBrush>| -> Vec<_> {
        layout.lines().map(|line| line.text_range()).collect()
    };

    let mut layout_ref = env.ranged_builder(text).build(text);
    layout_ref.break_all_lines(Some(20.0));
    assert_eq!(layout_ref.len(), 1);

    // A box anywhere in the word, including at either edge, leaves the line breaking unchanged.
    for index in [0, 5, text.len()] {
        let mut builder = env.ranged_builder(text);
        builder.push_floated_box(FloatedBox { id: 42, index });
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(20.0));

        assert_eq!(layout.width(), layout_ref.width(), "index {index}");
        assert_eq!(layout.height(), layout_ref.height(), "index {index}");
        assert_eq!(
            line_ranges(&layout),
            line_ranges(&layout_ref),
            "index {index}"
        );
    }
}

/// Every anchor is reported exactly once, on the line holding its text position, at the position
/// of the primary caret there.
#[test]
fn anchors_are_at_the_primary_caret() {
    use parley::Anchor;

    let mut env = TestEnv::new(test_name!(), None);
    for text in [
        "",
        "Hello world! This is a test of anchors.",
        "Hello\nworld\n",
        "abc אבג דהו 123 def",
        "אבג דהו abc 12 34",
        "1 2 34",
    ] {
        for rtl in [false, true] {
            for max_advance in [None, Some(60.0)] {
                let mut builder = env.ranged_builder(text);
                if rtl {
                    builder.set_base_direction(parley::BaseDirection::Rtl);
                }
                let indices: Vec<usize> = (0..=text.len())
                    .filter(|i| text.is_char_boundary(*i))
                    .collect();
                for &index in &indices {
                    // Two anchors per position
                    for id in [index * 2, index * 2 + 1] {
                        builder.push_anchor(Anchor {
                            id: id as u64,
                            index,
                        });
                    }
                }
                let mut layout: Layout<ColorBrush> = builder.build(text);
                layout.break_all_lines(max_advance);

                let mut seen = vec![0; (text.len() + 1) * 2];
                for (line_index, line) in layout.lines().enumerate() {
                    for item in line.items() {
                        let PositionedLayoutItem::Anchor(anchor) = item else {
                            continue;
                        };
                        seen[anchor.id as usize] += 1;
                        let range = line.text_range();
                        assert!(
                            range.start <= anchor.index && anchor.index <= range.end,
                            "{text:?}: anchor {} on line {range:?}",
                            anchor.index
                        );
                        let (caret_line, caret_x) = layout.primary_caret(anchor.index).unwrap();
                        // At a soft wrap the caret is on the next line, the anchor on this one.
                        if caret_line.text_range() == range {
                            assert!(
                                (caret_x - anchor.x).abs() < 0.01,
                                "{text:?} rtl={rtl} wrap={max_advance:?} line {line_index}: anchor {} at {}, caret at {caret_x}",
                                anchor.index,
                                anchor.x
                            );
                        }
                    }
                }
                for &index in &indices {
                    assert_eq!(seen[index * 2], 1, "{text:?} rtl={rtl}: anchor {index}");
                    assert_eq!(seen[index * 2 + 1], 1, "{text:?} rtl={rtl}: anchor {index}");
                }
            }
        }
    }
}
