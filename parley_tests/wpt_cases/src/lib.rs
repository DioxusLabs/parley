// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Text-layout fixtures ported from the Web Platform Tests, with Chrome as the oracle.
//!
//! A [`Case`] is one block container from a WPT page whose content is inline text
//! only: its content-box size, the computed values of the text properties on the
//! container and on each text node, and what Chrome laid out — the client rect of
//! every character ([`Case::chars`]). `parley_wpt_recorder` writes these; the
//! `parley_tests` WPT test lays each case out with Parley ([`layout()`]) and compares
//! ([`compare_chars`]).

mod compare;
mod format;
mod layout;
mod support;

use std::collections::BTreeMap;

pub use compare::{CharDiff, Mismatch, Tolerance, compare_chars};
pub use format::ParseError;
pub use layout::{
    CaseLayout, FontFiles, LAYOUT_UNIT, LayoutError, font_context, layout, parley_chars,
};
pub use support::{Unsupported, check_supported};

/// Computed CSS property values, keyed by property name.
pub type StyleMap = BTreeMap<String, String>;

/// A font the case's text resolved to, and the file it was served from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFace {
    /// The `@font-face` family name.
    pub family: String,
    /// File name within the fixture corpus' `fonts/` directory.
    pub file: String,
}

/// A text node and the computed style of its parent element.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// The text node's data, verbatim (white space not yet collapsed).
    pub text: String,
    /// The nearest ancestor `lang` attribute, or empty.
    pub lang: String,
    /// Index into [`Case::styles`].
    pub style: usize,
}

/// Chrome's client rect for one character, relative to the container's content box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharRect {
    /// Index of the run the character belongs to.
    pub run: usize,
    /// Index of the code point within the run.
    pub index: usize,
    /// Left edge, relative to the content box.
    pub x: f64,
    /// Top edge, relative to the content box.
    pub y: f64,
    /// Width in CSS px.
    pub width: f64,
    /// Height in CSS px.
    pub height: f64,
}

/// A Parley cluster's rect, attributed to its first code point, and how many code
/// points the cluster spans.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterRect {
    /// The rect, attributed to the cluster's first code point.
    pub rect: CharRect,
    /// Number of code points in the cluster.
    pub len: usize,
}

/// One WPT block container and what Chrome made of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    /// Path of the page within the WPT checkout.
    pub source: String,
    /// Index of the block among the page's exported blocks.
    pub block: usize,
    /// A CSS path locating the block in the page, for humans.
    pub path: String,
    /// Content-box width in CSS px.
    pub width: f64,
    /// Content-box height in CSS px.
    pub height: f64,
    /// The container's nearest `lang`.
    pub lang: String,
    /// Fonts referenced by the runs, in the order Chrome declared them.
    pub fonts: Vec<FontFace>,
    /// The container's computed text properties.
    pub container: StyleMap,
    /// Deduplicated run styles: only the values that differ from [`Self::container`].
    pub styles: Vec<StyleMap>,
    /// The text nodes, in document order.
    pub runs: Vec<Run>,
    /// Chrome's per-character rects, in run then character order. Characters with no
    /// rect (collapsed white space) are absent; a character may have several.
    pub chars: Vec<CharRect>,
}

impl Case {
    /// The concatenated text of every run.
    #[must_use]
    pub fn text(&self) -> String {
        self.runs.iter().map(|run| run.text.as_str()).collect()
    }

    /// A computed value on the container, or `""`.
    #[must_use]
    pub fn container_property(&self, name: &str) -> &str {
        self.container.get(name).map_or("", String::as_str)
    }

    /// A computed value on `run`'s style, falling back to the container's, or `""`.
    #[must_use]
    pub fn run_property(&self, run: &Run, name: &str) -> &str {
        self.styles[run.style]
            .get(name)
            .map_or_else(|| self.container_property(name), String::as_str)
    }
}
