// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Benchmarks for building text with the [`TreeBuilder`].

use crate::benches::chunks;
use crate::{ColorBrush, FONT_FAMILY_LIST, get_samples, with_contexts};
use parley::{FontFamily, FontWeight, StyleProperty, TextStyle, TreeBuilder, WhiteSpaceCollapse};
use std::hint::black_box;
use std::ops::Range;
use tango_bench::{Benchmark, benchmark_fn};

/// How text is split into [`TreeBuilder::push_text`] calls and style spans.
#[derive(Clone, Copy)]
struct Split {
    /// The number of characters in each `push_text` call.
    chars_per_push: usize,
    /// The number of `push_text` calls in each style span.
    pushes_per_span: usize,
}

const SPLITS: [Split; 3] = [
    Split {
        chars_per_push: 10,
        pushes_per_span: 1,
    },
    Split {
        chars_per_push: 10,
        pushes_per_span: 8,
    },
    Split {
        chars_per_push: 1,
        pushes_per_span: 10,
    },
];

const MODES: [WhiteSpaceCollapse; 2] = [WhiteSpaceCollapse::Collapse, WhiteSpaceCollapse::Preserve];

fn push_tree(
    builder: &mut TreeBuilder<'_, ColorBrush>,
    text: &str,
    ranges: &[Range<usize>],
    split: Split,
) {
    const STYLE: [StyleProperty<'static, ColorBrush>; 1] =
        [StyleProperty::FontWeight(FontWeight::BOLD)];
    for span in ranges.chunks(split.pushes_per_span) {
        builder.push_style_modification_span(&STYLE);
        for range in span {
            builder.push_text(&text[range.clone()]);
        }
        builder.pop_style_span();
    }
}

fn with_tree_builder<R>(
    mode: WhiteSpaceCollapse,
    f: impl FnOnce(TreeBuilder<'_, ColorBrush>) -> R,
) -> R {
    with_contexts(|font_cx, layout_cx| {
        let root_style = TextStyle {
            font_family: FontFamily::from(FONT_FAMILY_LIST),
            white_space_collapse: mode,
            ..TextStyle::default()
        };
        f(layout_cx.tree_builder(font_cx, 1.0, true, &root_style))
    })
}

/// Benchmark for pushing styled text to a [`TreeBuilder`], without building the layout.
///
/// All text is pushed into style spans, so white space processing is completed when the last span
/// is popped.
pub fn tree_builder_push() -> Vec<Benchmark> {
    let mut benchmarks = Vec::new();
    for sample in get_samples() {
        for split in SPLITS {
            for mode in MODES {
                let ranges: Vec<_> = chunks(&sample.text, split.chars_per_push).collect();
                benchmarks.push(benchmark_fn(
                    format!(
                        "TreeBuilder push - {} {} - {mode:?} - {} chars per push, {} pushes per span",
                        sample.name,
                        sample.modification,
                        split.chars_per_push,
                        split.pushes_per_span
                    ),
                    move |b| {
                        let ranges = ranges.clone();
                        b.iter(move || {
                            with_tree_builder(mode, |mut builder| {
                                push_tree(&mut builder, &sample.text, &ranges, split);
                                black_box(builder.text_so_far().text.len())
                            })
                        })
                    },
                ));
            }
        }
    }
    benchmarks
}

/// Benchmark for building a layout with a [`TreeBuilder`].
pub fn tree_builder_build() -> Vec<Benchmark> {
    let split = SPLITS[0];
    get_samples()
        .iter()
        .map(|sample| {
            let ranges: Vec<_> = chunks(&sample.text, split.chars_per_push).collect();
            benchmark_fn(
                format!(
                    "TreeBuilder build - {} {} - {} chars per push, {} pushes per span",
                    sample.name, sample.modification, split.chars_per_push, split.pushes_per_span
                ),
                move |b| {
                    let ranges = ranges.clone();
                    b.iter(move || {
                        with_tree_builder(WhiteSpaceCollapse::Collapse, |mut builder| {
                            push_tree(&mut builder, &sample.text, &ranges, split);
                            black_box(builder.build())
                        })
                    })
                },
            )
        })
        .collect()
}
