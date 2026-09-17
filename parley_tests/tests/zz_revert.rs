// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::util::*;
use parley::*;

#[test]
fn revert_restores_extents() {
    let mut env = TestEnv::new("zz_revert", None);
    let text = "aaa BBB";
    let mut builder = env.ranged_builder(text);
    builder.push(StyleProperty::FontSize(64.0), 4..7);
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(95.0));
    layout.align(Alignment::Start, AlignmentOptions::default());
    let h: Vec<f32> = (0..layout.len())
        .map(|i| layout.get(i).unwrap().metrics().line_height)
        .collect();
    eprintln!("HEIGHTS {h:?}");
    assert_eq!(layout.len(), 2);
    assert!(h[0] < h[1] / 2.0, "{h:?}");
}
