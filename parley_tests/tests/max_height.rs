// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for the line max-height constraint exposed via `BreakerState::set_line_max_height`.

use crate::test_name;
use crate::util::TestEnv;
use parley::{LineHeight, StyleProperty, YieldData};

const SMALL: f32 = 12.0;
const TALL: f32 = 40.0;

fn build_text(env: &mut TestEnv, text: &str) -> parley::Layout<crate::util::ColorBrush> {
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontSize(10.0));
    builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(SMALL)));
    // Use a distinct font size so the tall span shapes into its own run.
    let tall = text.find("tall").unwrap();
    builder.push(StyleProperty::FontSize(11.0), tall..tall + 4);
    builder.push(
        StyleProperty::LineHeight(LineHeight::Absolute(TALL)),
        tall..tall + 4,
    );
    builder.build(text)
}

/// Without a configured max height, tall content never yields `MaxHeightExceeded`.
#[test]
fn no_max_height_never_yields() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "aaa tall ccc";
    let mut layout = build_text(&mut env, text);

    let mut breaker = layout.break_lines();
    breaker.state_mut().set_layout_max_advance(1000.0);
    breaker.state_mut().set_line_max_advance(1000.0);
    while let Some(data) = breaker.break_next() {
        if let YieldData::MaxHeightExceeded(data) = data {
            panic!("unexpected max-height break at {}", data.line_height);
        }
    }
    breaker.finish();

    assert_eq!(layout.len(), 1);
    assert_eq!(layout.get(0).unwrap().metrics().line_height, TALL);
}

/// With a max height set, appending content taller than the limit yields `MaxHeightExceeded`
/// exactly once; reverting with a larger limit resumes normally.
#[test]
fn max_height_yields_once_and_resumes() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "aaa tall ccc";
    let mut layout = build_text(&mut env, text);

    let mut breaker = layout.break_lines();
    breaker.state_mut().set_layout_max_advance(1000.0);
    breaker.state_mut().set_line_max_advance(1000.0);
    breaker.state_mut().set_line_max_height(SMALL + 1.0);
    let checkpoint = breaker.state().clone();

    let mut yields = 0;
    while let Some(data) = breaker.break_next() {
        match data {
            YieldData::MaxHeightExceeded(data) => {
                yields += 1;
                assert_eq!(data.line_height, TALL);
                assert!(data.advance > 0.0);
                breaker.revert_to(checkpoint.clone());
                breaker.state_mut().set_line_max_height(TALL + 1.0);
            }
            YieldData::LineBreak(_) | YieldData::InlineBoxBreak(_) => {}
        }
    }
    breaker.finish();

    assert_eq!(yields, 1);
    assert_eq!(layout.len(), 1);
    assert_eq!(layout.get(0).unwrap().metrics().line_height, TALL);
}

/// A max height that fits all content behaves identically to no max height.
#[test]
fn max_height_not_exceeded_matches_unconstrained() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "aaa tall ccc aaa tall ccc aaa tall ccc";

    let mut unconstrained = build_text(&mut env, text);
    unconstrained.break_all_lines(Some(60.0));

    let mut constrained = build_text(&mut env, text);
    {
        let mut breaker = constrained.break_lines();
        breaker.state_mut().set_layout_max_advance(60.0);
        breaker.state_mut().set_line_max_advance(60.0);
        breaker.state_mut().set_line_max_height(TALL);
        while let Some(data) = breaker.break_next() {
            if let YieldData::MaxHeightExceeded(data) = data {
                panic!("unexpected max-height break at {}", data.line_height);
            }
        }
        breaker.finish();
    }

    assert!(unconstrained.len() > 1);
    assert_eq!(unconstrained.len(), constrained.len());
    for (a, b) in unconstrained.lines().zip(constrained.lines()) {
        assert_eq!(a.metrics().line_height, b.metrics().line_height);
        assert_eq!(a.metrics().advance, b.metrics().advance);
        assert_eq!(a.text_range(), b.text_range());
    }
}
