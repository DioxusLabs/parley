// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::BreakReason;
use crate::data::{LayoutData, LayoutItemKind};
use crate::layout::spacing::EffectiveSpacing;
use crate::style::Brush;

/// Alignment of a layout.
#[derive(Copy, Clone, Default, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Alignment {
    /// This is [`Alignment::Left`] for LTR text and [`Alignment::Right`] for RTL text.
    #[default]
    Start,
    /// This is [`Alignment::Right`] for LTR text and [`Alignment::Left`] for RTL text.
    End,
    /// Align content to the left edge.
    ///
    /// For alignment that should be aware of text direction, use [`Alignment::Start`] or
    /// [`Alignment::End`] instead.
    Left,
    /// Align each line centered within the container.
    Center,
    /// Align content to the right edge.
    ///
    /// For alignment that should be aware of text direction, use [`Alignment::Start`] or
    /// [`Alignment::End`] instead.
    Right,
    /// Justify each line by spacing out content.
    ///
    /// The last line of a paragraph is start-aligned instead, unless
    /// [`AlignmentOptions::last_line_alignment`] is set.
    Justify,
}

/// Additional options to fine tune alignment
#[derive(Debug, Clone, Copy)]
pub struct AlignmentOptions {
    /// If set to `true`, "end" and "center" alignment will apply even if the line contents are
    /// wider than the alignment width. If it is set to `false`, all overflowing lines will be
    /// [`Alignment::Start`] aligned.
    pub align_when_overflowing: bool,
    /// The alignment of the last line of each paragraph: the last line of the layout and every
    /// line that ends in an explicit line break. This corresponds to the CSS `text-align-last`
    /// property.
    ///
    /// If set to `None`, those lines use the alignment passed to [`Layout::align`], except that
    /// [`Alignment::Justify`] falls back to [`Alignment::Start`]. This is the behavior of
    /// `text-align-last: auto`.
    ///
    /// [`Layout::align`]: crate::Layout::align
    pub last_line_alignment: Option<Alignment>,
}

#[expect(
    clippy::derivable_impls,
    reason = "Make default values explicit rather than relying on the implicit default value of bool"
)]
impl Default for AlignmentOptions {
    fn default() -> Self {
        Self {
            align_when_overflowing: false,
            last_line_alignment: None,
        }
    }
}

/// Align the layout.
pub(crate) fn align<B: Brush>(
    layout: &mut LayoutData<B>,
    alignment: Alignment,
    options: AlignmentOptions,
) {
    layout.alignment = Some(alignment);

    let is_rtl = layout.base_level.is_rtl();

    // Apply alignment to line items
    let mut next_box = 0;
    for (line_index, line) in layout.lines.iter_mut().enumerate() {
        line.justification.amount_per_opportunity = 0.;

        let indent = line.indent;

        if is_rtl {
            // In RTL text, trailing whitespace is on the left. As we hang that whitespace, offset
            // the line to the left. Note: indent is not subtracted here because `free_space` below
            // already accounts for it.
            line.metrics.offset = -line.metrics.hanging_advance;
        } else {
            line.metrics.offset = indent;
        }

        // Compute free space.
        let line_width = line.metrics.inline_max_coord - line.metrics.inline_min_coord;
        let free_space = line_width - indent - line.metrics.advance + line.metrics.hanging_advance;

        if !options.align_when_overflowing && free_space <= 0.0 {
            if is_rtl {
                // In RTL text, right-align on overflow.
                line.metrics.offset += free_space;
            }
        } else {
            let is_last_line =
                matches!(line.break_reason, BreakReason::None | BreakReason::Explicit);
            let line_alignment = if !is_last_line {
                alignment
            } else if let Some(last_line_alignment) = options.last_line_alignment {
                last_line_alignment
            } else if alignment == Alignment::Justify {
                Alignment::Start
            } else {
                alignment
            };

            match (line_alignment, is_rtl) {
                (Alignment::Left, _) | (Alignment::Start, false) | (Alignment::End, true) => {}
                (Alignment::Right, _) | (Alignment::Start, true) | (Alignment::End, false) => {
                    line.metrics.offset += free_space;
                }
                (Alignment::Center, _) => {
                    line.metrics.offset += free_space * 0.5;
                }
                (Alignment::Justify, _) => {
                    if free_space > 0.0 {
                        if line.num_justification_opportunities == 0 {
                            if is_rtl {
                                line.metrics.offset += free_space;
                            }
                        } else {
                            line.justification.amount_per_opportunity =
                                free_space / line.num_justification_opportunities as f32;
                        }
                    }
                }
            }
        }

        let first_box = next_box;
        while next_box < layout.positioned_inline_box_indices.len()
            && layout.inline_boxes[layout.positioned_inline_box_indices[next_box]].line_index
                == line_index
        {
            next_box += 1;
        }
        if first_box == next_box {
            continue;
        }
        let box_indices = &layout.positioned_inline_box_indices[first_box..next_box];
        if line.justification.amount_per_opportunity == 0. {
            for &index in box_indices {
                layout.inline_boxes[index].justification_x = 0.;
            }
        } else {
            let mut advance = 0.;
            for item in &layout.line_items[line.item_range.clone()] {
                match item.kind {
                    LayoutItemKind::TextRun => {
                        let spacing = EffectiveSpacing::new(
                            layout.runs[item.index].spacing,
                            line.justification,
                        );
                        let slice = layout
                            .shaped_text
                            .run_slice(item.index as u32)
                            .narrow(item.shaped_cluster_range.clone());
                        advance += spacing.slice_advance(slice);
                    }
                    LayoutItemKind::InlineBox => {
                        let inline_box = &mut layout.inline_boxes[item.index];
                        inline_box.justification_x = advance - inline_box.x;
                        if inline_box.inline_box.kind == crate::InlineBoxKind::InFlow {
                            advance += inline_box.inline_box.width;
                        }
                    }
                }
            }
        }
    }
}
