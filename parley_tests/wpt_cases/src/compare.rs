// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Comparing Parley's cluster rects with Chrome's character rects.

use crate::layout::LAYOUT_UNIT;
use crate::{Case, CharRect, ClusterRect};

/// How far apart two coordinates may be and still count as equal, in CSS px.
///
/// Chrome's client rects are snapped to `LayoutUnit`s (1/64 px), its glyph advances
/// are accumulated in 16.16 fixed point and alignment offsets are computed from the
/// snapped line width, so positions routinely differ from exact arithmetic by a few
/// `LayoutUnit`s. The default allows 1/16 px: far below what any WPT test asserts, since
/// a character on the wrong line or the wrong side of a space is off by whole glyphs.
/// Each text fragment (roughly: each text node) on a line is snapped separately, so a
/// character's `x` may be off by one more `LayoutUnit` per fragment before it on the
/// line ([`Tolerance::per_fragment`]).
///
/// In the block axis Chrome rounds a font's ascent and descent to whole pixels and
/// floors the half-leading, so `y` and `height` are only checked to the pixel: enough
/// to tell which line a character is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    /// For `width`, and the base for `x`.
    pub inline: f64,
    /// Added to the `x` tolerance for every fragment before the character on its line.
    pub per_fragment: f64,
    /// For `y` and `height`.
    pub block: f64,
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            inline: 4.0 * LAYOUT_UNIT + 1e-3,
            per_fragment: LAYOUT_UNIT,
            block: 1.0 + 1e-3,
        }
    }
}

/// One character whose rects disagree.
#[derive(Clone, Debug, PartialEq)]
pub struct CharDiff {
    /// Index of the run the character belongs to.
    pub run: usize,
    /// Index of the code point within the run.
    pub index: usize,
    /// The character, for the report.
    pub character: Option<char>,
    /// Parley's rect, if it laid the character out.
    pub parley: Option<CharRect>,
    /// Chrome's rect, if it laid the character out.
    pub chrome: Option<CharRect>,
}

/// Characters Chrome laid out that Parley did not, or vice versa, or that landed
/// somewhere else.
#[derive(Clone, Debug, PartialEq)]
pub struct Mismatch {
    /// At most [`Mismatch::MAX_REPORTED`] diffs.
    pub diffs: Vec<CharDiff>,
    /// Total number of differing characters.
    pub count: usize,
}

impl Mismatch {
    /// How many diffs a report lists before truncating.
    pub const MAX_REPORTED: usize = 12;
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{} character(s) differ:", self.count)?;
        writeln!(
            f,
            "  {:<10} {:<6} {:>34}   {:>34}",
            "run:index", "char", "parley (x y w h)", "chrome (x y w h)"
        )?;
        for diff in &self.diffs {
            let character = diff
                .character
                .map_or_else(|| "?".to_string(), |c| format!("{:?}", c.to_string()));
            writeln!(
                f,
                "  {:<10} {:<6} {:>34}   {:>34}",
                format!("{}:{}", diff.run, diff.index),
                character,
                rect(diff.parley),
                rect(diff.chrome)
            )?;
        }
        if self.count > self.diffs.len() {
            writeln!(f, "  ... and {} more", self.count - self.diffs.len())?;
        }
        Ok(())
    }
}

fn rect(rect: Option<CharRect>) -> String {
    match rect {
        None => "-".to_string(),
        Some(r) => format!(
            "{} {} {} {}",
            trim(r.x),
            trim(r.y),
            trim(r.width),
            trim(r.height)
        ),
    }
}

fn trim(value: f64) -> String {
    let mut text = format!("{value:.4}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

fn union(a: &mut CharRect, b: &CharRect) {
    let right = (a.x + a.width).max(b.x + b.width);
    let bottom = (a.y + a.height).max(b.y + b.height);
    a.x = a.x.min(b.x);
    a.y = a.y.min(b.y);
    a.width = right - a.x;
    a.height = bottom - a.y;
}

/// Chrome's rects reduced to Parley's cluster granularity: the rects of a cluster's
/// code points (its first plus `len - 1` continuations, where Chrome gave them any)
/// are joined into one bounding rect keyed by the first code point.
///
/// A code point Parley knows no cluster for keeps its own rect and will be reported
/// as missing from Parley. Empty rects are Chrome's way of saying a character was not
/// laid out (collapsed or hanging white space, a newline) and are dropped.
fn chrome_clusters(case: &Case, parley: &[ClusterRect]) -> Vec<CharRect> {
    let mut clusters: Vec<CharRect> = Vec::with_capacity(case.chars.len());
    let mut parley = parley.iter().peekable();
    for &c in case.chars.iter().filter(|c| has_width(c)) {
        // Advance to the Parley cluster that could contain this code point.
        while parley.peek().is_some_and(|p| {
            cluster_end(p) <= (c.run, c.index) && (p.rect.run, p.rect.index) < (c.run, c.index)
        }) {
            parley.next();
        }
        let key = match parley.peek() {
            Some(p)
                if (p.rect.run, p.rect.index) <= (c.run, c.index)
                    && (c.run, c.index) < cluster_end(p) =>
            {
                (p.rect.run, p.rect.index)
            }
            _ => (c.run, c.index),
        };
        match clusters.last_mut() {
            Some(last) if (last.run, last.index) == key => union(last, &c),
            _ => clusters.push(CharRect {
                run: key.0,
                index: key.1,
                ..c
            }),
        }
    }
    clusters
}

/// Whether a rect is more than a snapping artefact wide: a character whose advance
/// was spaced away to nothing is reported by Chrome as up to a `LayoutUnit` wide.
fn has_width(rect: &CharRect) -> bool {
    rect.width > LAYOUT_UNIT + 1e-3
}

/// The `(run, index)` one past the last code point of `cluster`. Clusters never span
/// runs (a run boundary is a style boundary).
fn cluster_end(cluster: &ClusterRect) -> (usize, usize) {
    (cluster.rect.run, cluster.rect.index + cluster.len)
}

/// How many runs other than `rect.run` have a character on `rect`'s line before it in
/// DOM order, i.e. how many text fragments Chrome laid out before this one.
fn fragments_before(chrome: &[CharRect], rect: &CharRect, tolerance: Tolerance) -> usize {
    let mut runs: Vec<usize> = chrome
        .iter()
        .filter(|c| c.run < rect.run && (c.y - rect.y).abs() <= tolerance.block)
        .map(|c| c.run)
        .collect();
    runs.dedup();
    runs.len()
}

/// Compares Parley's clusters ([`crate::parley_chars`]) with Chrome's rects.
///
/// Clusters without an advance are treated like Chrome's empty rects: not laid out.
pub fn compare_chars(
    case: &Case,
    parley: &[ClusterRect],
    tolerance: Tolerance,
) -> Result<(), Mismatch> {
    let chrome = chrome_clusters(case, parley);
    let character = |run: usize, index: usize| case.runs[run].text.chars().nth(index);

    let mut parley = parley.iter().map(|c| c.rect).filter(has_width).peekable();
    let chrome_rects = chrome.as_slice();
    let mut chrome = chrome.iter().copied().peekable();
    let mut diffs = Vec::new();
    let mut count = 0;
    let mut report = |diff: CharDiff| {
        count += 1;
        if diffs.len() < Mismatch::MAX_REPORTED {
            diffs.push(diff);
        }
    };

    loop {
        let key = |r: &CharRect| (r.run, r.index);
        let (p, c) = match (parley.peek().copied(), chrome.peek().copied()) {
            (None, None) => break,
            (Some(p), Some(c)) if key(&p) == key(&c) => {
                parley.next();
                chrome.next();
                (Some(p), Some(c))
            }
            (Some(p), Some(c)) if key(&p) < key(&c) => {
                parley.next();
                (Some(p), None)
            }
            (Some(p), None) => {
                parley.next();
                (Some(p), None)
            }
            (_, Some(c)) => {
                chrome.next();
                (None, Some(c))
            }
        };
        let matches = match (&p, &c) {
            (Some(p), Some(c)) => {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a line has far fewer fragments than 2^52"
                )]
                let slack =
                    tolerance.per_fragment * fragments_before(chrome_rects, c, tolerance) as f64;
                rects_match(p, c, tolerance, slack)
            }
            _ => false,
        };
        if !matches {
            let (run, index) = key(&p.or(c).expect("one side is present"));
            report(CharDiff {
                run,
                index,
                character: character(run, index),
                parley: p,
                chrome: c,
            });
        }
    }

    if count == 0 {
        Ok(())
    } else {
        Err(Mismatch { diffs, count })
    }
}

fn rects_match(parley: &CharRect, chrome: &CharRect, tolerance: Tolerance, x_slack: f64) -> bool {
    (parley.x - chrome.x).abs() <= tolerance.inline + x_slack
        && (parley.width - chrome.width).abs() <= tolerance.inline
        && (parley.y - chrome.y).abs() <= tolerance.block
        && (parley.height - chrome.height).abs() <= tolerance.block
}
