// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Context for layout.

use super::FontContext;
use super::context::LayoutContext;
use super::style::{Brush, StyleProperty, TextStyle};

use super::layout::Layout;

use alloc::string::String;
use core::ops::RangeBounds;
use parlance::BaseDirection;
use parley_engine::break_overrides::LineBreakOverrideFn;
use parley_engine::{AnalysisDataSources, DictionaryProvider, InlineObject};

use crate::InlineBoxKind;
use crate::inline_box::{InlineBox, LayoutInlineBox};
use crate::resolve::tree::TreeStyleBuilder;
use crate::resolve::{RangedStyleBuilder, ResolvedProperty, ResolvedStyle, StyleRun};
use crate::util::{resolve_bounded_range, resolve_range};

#[derive(Clone, Copy)]
pub(crate) struct BuilderOptions<'a> {
    scale: f32,
    quantize: bool,
    base_direction: BaseDirection,
    line_break_override: Option<&'a LineBreakOverrideFn>,
    dictionary_provider: Option<&'a dyn DictionaryProvider>,
}

impl BuilderOptions<'_> {
    pub(crate) fn new(scale: f32, quantize: bool) -> Self {
        Self {
            scale,
            quantize,
            base_direction: BaseDirection::Auto,
            line_break_override: None,
            dictionary_provider: None,
        }
    }
}

/// Builder for constructing a text layout with ranged attributes.
///
/// This builder does not collapse whitespace. For whitespace collapsing, use the [`TreeBuilder`]
#[must_use]
pub struct RangedBuilder<'a, B: Brush> {
    /// The style run builder that the resolved styles, runs and inline boxes are pushed to.
    pub(crate) sink: StyleRunBuilder<'a, B>,
    /// Taken from the [`LayoutContext`] while building, and returned to it when dropped.
    pub(crate) ranged: RangedStyleBuilder<B>,
}

impl<'b, B: Brush> RangedBuilder<'b, B> {
    pub fn push_default<'a>(&mut self, property: impl Into<StyleProperty<'a, B>>) {
        let resolved = self.sink.resolve_property(&property.into());
        self.ranged.push_default(resolved);
    }

    pub fn push<'a>(
        &mut self,
        property: impl Into<StyleProperty<'a, B>>,
        range: impl RangeBounds<usize>,
    ) {
        let resolved = self.sink.resolve_property(&property.into());
        self.ranged.push(resolved, range);
    }

    /// Adds an inline box.
    ///
    /// In-flow boxes ([`InlineBoxKind::InFlow`]) must have a matching U+FFFC OBJECT REPLACEMENT
    /// CHARACTER at its [`index`](InlineBox::index) in the text.
    ///
    /// Out-of-flow boxes are not represented in the text.
    ///
    /// # Panics
    ///
    /// Building the layout panics if an in-flow box's index isn't that of a U+FFFC character in the
    /// text, or if two in-flow boxes share a single U+FFFC character.
    pub fn push_inline_box(&mut self, inline_box: InlineBox) {
        self.sink.push_inline_box(inline_box);
    }

    /// Sets the paragraph's base direction.
    ///
    /// The default is [`BaseDirection::Auto`], which infers the direction from the text.
    pub fn set_base_direction(&mut self, base_direction: BaseDirection) {
        self.sink.set_base_direction(base_direction);
    }

    /// Set the callback which will be called as a first provider of line breaking decisions.
    ///
    /// See [`LineBreakOverrideFn`] for more details.
    pub fn set_line_break_override(&mut self, overrides: Option<&'b LineBreakOverrideFn>) {
        self.sink.set_line_break_override(overrides);
    }

    /// Set the provider for dictionary data used for word and line segmentation.
    ///
    /// See [`DictionaryProvider`] for more details.
    ///
    /// # Panics
    ///
    /// Building this layout may panic if the provider returns an error other than `IdentifierNotFound`.
    pub fn set_dictionary_provider(&mut self, provider: Option<&'b dyn DictionaryProvider>) {
        self.sink.set_dictionary_provider(provider);
    }

    pub fn build_into(mut self, layout: &mut Layout<B>, text: impl AsRef<str>) {
        self.ranged.finish(&mut self.sink);
        self.sink.finish_into(layout, text.as_ref());
    }

    pub fn build(self, text: impl AsRef<str>) -> Layout<B> {
        let mut layout = Layout::default();
        self.build_into(&mut layout, text);
        layout
    }
}

impl<B: Brush> Drop for RangedBuilder<'_, B> {
    fn drop(&mut self) {
        self.sink.lcx.ranged_style_builder = core::mem::take(&mut self.ranged);
    }
}

/// Builder for constructing a text layout from a style table and
/// indexed style runs.
///
/// This builder does not collapse whitespace. For whitespace collapsing, use the [`TreeBuilder`]
#[must_use]
pub struct StyleRunBuilder<'a, B: Brush> {
    pub(crate) options: BuilderOptions<'a>,
    /// The length of the text, if it was supplied up front.
    pub(crate) len: Option<usize>,
    pub(crate) lcx: &'a mut LayoutContext<B>,
    pub(crate) fcx: &'a mut FontContext,
    pub(crate) cursor: usize,
}

impl<'b, B: Brush> StyleRunBuilder<'b, B> {
    /// Reserves additional capacity for styles and runs.
    ///
    /// This is an optional optimization for callers that know counts
    /// up front; call it before pushing styles and runs to reduce
    /// reallocations.
    pub fn reserve(&mut self, additional_styles: usize, additional_runs: usize) {
        self.lcx.style_table.reserve(additional_styles);
        self.lcx.style_runs.reserve(additional_runs);
    }

    /// Adds a fully-specified style to the shared style table and
    /// returns its index.
    ///
    /// The style is treated as a child of the root style (index 0); use
    /// [`Self::push_child_style`] to nest it in another span.
    pub fn push_style<'family, 'settings>(
        &mut self,
        style: TextStyle<'family, 'settings, B>,
    ) -> u16 {
        self.push_child_style(0, style)
    }

    /// Adds a fully-specified style nested in the span whose style has index
    /// `parent_style_index`, and returns its index. The parent is what the
    /// style's `vertical_align` is relative to.
    ///
    /// # Panics
    ///
    /// If `parent_style_index` is not `0` or the index of a previously added style.
    pub fn push_child_style<'family, 'settings>(
        &mut self,
        parent_style_index: u16,
        style: TextStyle<'family, 'settings, B>,
    ) -> u16 {
        let resolved = self
            .lcx
            .rcx
            .resolve_entire_style_set(self.fcx, &style, self.options.scale);
        self.push_resolved_style(parent_style_index, resolved)
    }

    /// Resolves `style` against this builder's scale, without adding it to the style table.
    pub(crate) fn resolve_style(&mut self, style: &TextStyle<'_, '_, B>) -> ResolvedStyle<B> {
        self.lcx
            .rcx
            .resolve_entire_style_set(self.fcx, style, self.options.scale)
    }

    /// Resolves a single `property` against this builder's scale.
    pub(crate) fn resolve_property(
        &mut self,
        property: &StyleProperty<'_, B>,
    ) -> ResolvedProperty<B> {
        self.lcx
            .rcx
            .resolve_property(self.fcx, property, self.options.scale)
    }

    /// Adds an already resolved style with the given parent to the style table and returns
    /// its index.
    pub(crate) fn push_resolved_style(
        &mut self,
        parent_style_index: u16,
        mut style: ResolvedStyle<B>,
    ) -> u16 {
        let style_index = self.lcx.style_table.len();
        assert!(
            parent_style_index == 0 || usize::from(parent_style_index) < style_index,
            "StyleRunBuilder expects a parent style that was previously added"
        );
        assert!(style_index <= u16::MAX as usize, "too many styles");
        style.parent = parent_style_index;
        self.lcx.style_table.push(style);
        style_index as u16
    }

    /// Adds a style run referencing an entry from the style table.
    ///
    /// Runs must be contiguous and non-overlapping, and must cover
    /// `0..text.len()` once all runs have been added. If the builder was
    /// created without the text, the range's end must be bounded. A run
    /// that continues the previous run with the same style is merged into it.
    pub fn push_style_run(&mut self, style_index: u16, range: impl RangeBounds<usize>) {
        let range = match self.len {
            Some(len) => resolve_range(range, len),
            None => resolve_bounded_range(range).expect(
                "StyleRunBuilder expects an explicit run end when the text is not known up front",
            ),
        };
        assert!(
            range.start == self.cursor,
            "StyleRunBuilder expects contiguous non-overlapping runs"
        );
        assert!(
            range.start <= range.end,
            "StyleRunBuilder expects ordered ranges"
        );
        assert!(
            (style_index as usize) < self.lcx.style_table.len(),
            "StyleRunBuilder expects style indices that were previously added via push_style"
        );
        match self.lcx.style_runs.last_mut() {
            Some(run) if run.style_index == style_index && run.range.end == range.start => {
                run.range.end = range.end;
            }
            _ => self.lcx.style_runs.push(StyleRun {
                style_index,
                range: range.clone(),
            }),
        }
        self.cursor = range.end;
    }

    /// Adds an inline box, treated as a child of the root style (index 0); use
    /// [`Self::push_child_inline_box`] to nest it in another span.
    ///
    /// In-flow boxes ([`InlineBoxKind::InFlow`]) must have a matching U+FFFC OBJECT REPLACEMENT
    /// CHARACTER at its [`index`](InlineBox::index) in the text.
    ///
    /// Out-of-flow boxes are not represented in the text.
    ///
    /// # Panics
    ///
    /// Building the layout panics if an in-flow box's index isn't that of a U+FFFC character in the
    /// text, or if two in-flow boxes share a single U+FFFC character.
    pub fn push_inline_box(&mut self, inline_box: InlineBox) {
        self.push_child_inline_box(0, inline_box);
    }

    /// Adds an inline box nested in the span whose style has index `parent_style_index`,
    /// which is what the box's `vertical_align` is relative to. For an in-flow box the parent
    /// is instead the style of the run containing its U+FFFC character.
    ///
    /// # Panics
    ///
    /// If `parent_style_index` is not `0` or the index of a previously added style.
    pub fn push_child_inline_box(&mut self, parent_style_index: u16, inline_box: InlineBox) {
        assert!(
            parent_style_index == 0 || usize::from(parent_style_index) < self.lcx.style_table.len(),
            "StyleRunBuilder expects a parent style that was previously added"
        );
        self.lcx
            .inline_boxes
            .push(LayoutInlineBox::new(inline_box, parent_style_index));
    }

    /// Sets the paragraph's base direction.
    ///
    /// The default is [`BaseDirection::Auto`], which infers the direction from the text.
    pub fn set_base_direction(&mut self, base_direction: BaseDirection) {
        self.options.base_direction = base_direction;
    }

    /// Set the callback which will be called as a first provider of line breaking decisions.
    ///
    /// See [`LineBreakOverrideFn`] for more details.
    pub fn set_line_break_override(&mut self, overrides: Option<&'b LineBreakOverrideFn>) {
        self.options.line_break_override = overrides;
    }

    /// Set the provider for dictionary data used for word and line segmentation.
    ///
    /// See [`DictionaryProvider`] for more details.
    ///
    /// # Panics
    ///
    /// Building this layout may panic if the provider returns an error other than `IdentifierNotFound`.
    pub fn set_dictionary_provider(&mut self, provider: Option<&'b dyn DictionaryProvider>) {
        self.options.dictionary_provider = provider;
    }

    pub fn build_into(mut self, layout: &mut Layout<B>, text: impl AsRef<str>) {
        self.finish_into(layout, text.as_ref());
    }

    /// Checks that the runs cover `text` and builds the layout, leaving the builder spent.
    pub(crate) fn finish_into(&mut self, layout: &mut Layout<B>, text: &str) {
        assert!(
            self.cursor == text.len(),
            "StyleRunBuilder requires runs that cover the full text"
        );
        if self.lcx.style_runs.is_empty() {
            // This is reachable for empty text. In this case, the layout still needs a style and an
            // empty style run, e.g. to size a cursor.
            if self.lcx.style_table.is_empty() {
                let style = self.lcx.rcx.resolve_entire_style_set(
                    self.fcx,
                    &TextStyle::default(),
                    self.options.scale,
                );
                self.lcx.style_table.push(style);
            }
            self.lcx.style_runs.push(StyleRun {
                style_index: 0,
                range: 0..0,
            });
        }
        build_into_layout(layout, text, self.lcx, self.fcx, self.options);
    }

    pub fn build(self, text: impl AsRef<str>) -> Layout<B> {
        let mut layout = Layout::default();
        self.build_into(&mut layout, text);
        layout
    }
}

/// Builder for constructing a text layout with a tree of attributes.
#[must_use]
pub struct TreeBuilder<'a, B: Brush> {
    /// The style run builder that the tree's styles, runs and inline boxes are pushed to.
    pub(crate) sink: StyleRunBuilder<'a, B>,
    /// Taken from the [`LayoutContext`] while building, and returned to it when dropped.
    pub(crate) tree: TreeStyleBuilder<B>,
}

impl<'b, B: Brush> TreeBuilder<'b, B> {
    pub fn push_style_span(&mut self, style: TextStyle<'_, '_, B>) {
        let resolved = self.sink.resolve_style(&style);
        self.tree.push_style_span(resolved);
    }

    pub fn push_style_modification_span<'s, 'iter>(
        &mut self,
        properties: impl IntoIterator<Item = &'iter StyleProperty<'s, B>>,
    ) where
        's: 'iter,
        B: 'iter,
    {
        self.tree.push_style_modification_span(
            properties
                .into_iter()
                .map(|p| self.sink.resolve_property(p)),
        );
    }

    pub fn pop_style_span(&mut self) {
        self.tree.pop_style_span();
    }

    pub fn push_text(&mut self, text: &str) {
        self.tree.push_text(text, &mut self.sink);
    }

    /// Returns the pushed text so far, after whitespace processing, but excluding any trailing
    /// collapsible whitespace.
    ///
    /// An in-flow inline box ([`InlineBoxKind::InFlow`]) appears in the text as a U+FFFC OBJECT
    /// REPLACEMENT CHARACTER (see [`Self::push_inline_box`]).
    ///
    /// This is a prefix of the text that [`Self::build`] will return. Whether that trailing
    /// collapsible whitespace ends up being kept (as a single space) depends on where it is and on
    /// what follows it. See [`Self::has_pending_whitespace`].
    pub fn text(&self) -> &str {
        self.tree.text()
    }

    /// Returns whether [`Self::text`] is followed by collapsible whitespace.
    ///
    /// That white space is not yet part of [`Self::text`]: depending on where it is and what
    /// follows, it is either removed or kept as a single space.
    pub fn has_pending_whitespace(&self) -> bool {
        self.tree.has_pending_whitespace()
    }

    /// Pushes an inline box at the current position in the text.
    ///
    /// Note: this overwrites the box's [`InlineBox::index`].
    ///
    /// Pushing an in-flow box ([`InlineBoxKind::InFlow`]) appends a `U+FFFC` OBJECT REPLACEMENT
    /// CHARACTER to the text that [`Self::build`] will return.
    ///
    /// [atomic inline]: https://www.w3.org/TR/css-display-3/#atomic-inline
    pub fn push_inline_box(&mut self, mut inline_box: InlineBox) {
        // TODO: arrange type better here to factor out the index
        inline_box.index = if inline_box.kind == InlineBoxKind::InFlow {
            self.tree.push_inline_object(&mut self.sink)
        } else {
            self.tree.text().len()
        };
        let parent_style_index = self.tree.resolve_current_style_id(&mut self.sink);
        self.sink
            .push_child_inline_box(parent_style_index, inline_box);
    }

    /// Sets the paragraph's base direction.
    ///
    /// The default is [`BaseDirection::Auto`], which infers the direction from the text.
    pub fn set_base_direction(&mut self, base_direction: BaseDirection) {
        self.sink.set_base_direction(base_direction);
    }

    /// Set the callback which will be called as a first provider of line breaking decisions.
    ///
    /// See [`LineBreakOverrideFn`] for more details.
    pub fn set_line_break_override(&mut self, overrides: Option<&'b LineBreakOverrideFn>) {
        self.sink.set_line_break_override(overrides);
    }

    /// Set the provider for dictionary data used for word and line segmentation.
    ///
    /// See [`DictionaryProvider`] for more details.
    ///
    /// # Panics
    ///
    /// Building this layout may panic if the provider returns an error other than `IdentifierNotFound`.
    pub fn set_dictionary_provider(&mut self, provider: Option<&'b dyn DictionaryProvider>) {
        self.sink.set_dictionary_provider(provider);
    }

    #[inline]
    pub fn build_into(mut self, layout: &mut Layout<B>) -> String {
        let text = self.tree.finish();
        self.sink.finish_into(layout, &text);
        text
    }

    #[inline]
    pub fn build(self) -> (Layout<B>, String) {
        let mut layout = Layout::default();
        let text = self.build_into(&mut layout);
        (layout, text)
    }
}

impl<B: Brush> Drop for TreeBuilder<'_, B> {
    fn drop(&mut self) {
        self.sink.lcx.tree_style_builder = core::mem::take(&mut self.tree);
    }
}

fn build_into_layout<B: Brush>(
    layout: &mut Layout<B>,
    text: &str,
    lcx: &mut LayoutContext<B>,
    fcx: &mut FontContext,
    options: BuilderOptions<'_>,
) {
    if text.is_empty() && lcx.style_runs.is_empty() {
        lcx.style_table.push(ResolvedStyle::default());
        lcx.style_runs.push(StyleRun {
            style_index: 0,
            range: 0..0,
        });
    }
    assert!(
        !lcx.style_runs.is_empty(),
        "at least one style run is required"
    );

    // Sort the inline boxes as subsequent code assumes that they are in text index order.
    // Note: It's important that this is a stable sort to allow users to control the order of contiguous inline boxes
    //
    // TODO: consider dropping the sort and requiring `push_inline_box` callers to push boxes in text index order
    // (as `TreeBuilder` already does).
    lcx.inline_boxes.sort_by_key(|b| b.inline_box.index);

    lcx.inline_objects.clear();
    for inline_box in &mut lcx.inline_boxes {
        if !inline_box.is_inline_object() {
            continue;
        }
        let index = inline_box.inline_box.index;
        assert!(
            text.get(index..)
                .is_some_and(|rest| rest.starts_with('\u{FFFC}')),
            "in-flow inline box {} must be at a U+FFFC OBJECT REPLACEMENT CHARACTER in the text, \
             but its index is {index}",
            inline_box.inline_box.id,
        );
        assert!(
            lcx.inline_objects
                .last()
                .map(|object| object.text_byte_start)
                != Some(index),
            "in-flow inline boxes must each have a U+FFFC of their own, but more than one is at \
             index {index}",
        );
        lcx.inline_objects.push(InlineObject {
            text_byte_start: index,

            // TODO: We may want to follow CSS atomic inline's break behavior more closely.
            // See <https://www.w3.org/TR/css-text-4/#atomic-compat-wrap>.
            break_before: Some(true),
            break_after: Some(true),
        });

        // The box's parent is the span its U+FFFC character is in.
        let run = lcx.style_runs.partition_point(|run| run.range.end <= index);
        inline_box.parent_style_index = lcx.style_runs[run].style_index;
    }

    let data_sources = AnalysisDataSources::new(options.dictionary_provider);

    crate::analysis::analyze_text(
        lcx,
        text,
        options.base_direction,
        options.line_break_override,
        &data_sources,
    );

    layout.data.clear();
    layout.data.scale = options.scale;
    layout.data.quantize = options.quantize;
    layout.data.base_level = lcx.analysis.paragraph_level();
    layout.data.text_len = text.len();

    // Characters not covered by any style run (e.g. the space substituted for empty text during
    // analysis) use the root style.
    lcx.char_style_indices.clear();
    lcx.char_style_indices
        .resize(lcx.analysis.char_info().len(), 0);
    let mut char_index = 0;
    for style_run in &lcx.style_runs {
        for _ in text[style_run.range.clone()].chars() {
            lcx.char_style_indices[char_index] = style_run.style_index;
            char_index += 1;
        }
    }

    // Copy the visual styles into the layout
    layout
        .data
        .styles
        .extend(lcx.style_table.iter().map(|s| s.as_layout_style()));

    {
        super::shape::shape_text(
            &lcx.rcx,
            fcx,
            &lcx.style_table,
            &lcx.inline_boxes,
            &lcx.analysis,
            &lcx.char_style_indices,
            &mut lcx.scx,
            text,
            layout,
        );
    }

    // After shaping, so that the metrics of fonts used for shaping can be reused.
    crate::layout::style_metrics::resolve_style_metrics(
        &lcx.rcx,
        fcx,
        &lcx.style_table,
        &layout.data.shaped_text,
        options.quantize,
        &mut layout.data.style_metrics,
    );

    // Move inline boxes into the layout
    layout.data.inline_boxes.clear();
    core::mem::swap(&mut layout.data.inline_boxes, &mut lcx.inline_boxes);

    // Find the shaped clusters of the inline objects, and place their boxes there.
    //
    // TODO: Maybe `parley_engine` should expose a utility to easily get indices of shaped clusters?
    for box_index in 0..layout.data.inline_boxes.len() {
        let layout_box = &layout.data.inline_boxes[box_index];
        if !layout_box.is_inline_object() {
            continue;
        }
        let index = layout_box.inline_box.index;
        let characters = layout.data.shaped_text.characters();
        let clusters = layout.data.shaped_text.shaped_clusters();
        let char_index =
            characters.partition_point(|c| (c.text_byte_start as usize) < index) as u32;
        let cluster = clusters.partition_point(|c| c.chars_range().start < char_index);
        // Each object is a cluster of its own, but there is none only if shaping was aborted due to
        // lack of fonts.
        layout.data.inline_boxes[box_index].inline_object_cluster =
            (cluster < clusters.len()).then_some(cluster as u32);
        layout.data.place_inline_object(box_index);
    }
}
