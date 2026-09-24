// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Itemization breaks text into individually-shapeable items.

use core::ops::Range;

use icu_properties::props::Script as IcuScript;
use parlance::{BidiLevel, Script};

use crate::{Analysis, CharInfo, ShapeOptions};

/// A range of text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRange {
    /// The range of byte offsets.
    pub byte_range: Range<usize>,

    /// The range of character indices.
    pub char_range: Range<usize>,
}

/// A span of text inside an [`Item`] with constant script and bidirectional embedding level.
#[derive(Clone, Debug)]
pub struct Segment {
    /// The text range of this item.
    pub range: TextRange,

    /// The bidi level of characters in this item.
    pub bidi_level: BidiLevel,

    /// The script of characters in this item.
    ///
    /// Characters in the source text that do not have a particular script (i.e., they are one of
    /// [`Script::COMMON`], [`Script::UNKNOWN`] or [`Script::INHERITED`]) get their script from
    /// surrounding context. Currently, these just inherit the script of the preceding characters.
    /// Leading characters without a particular script inherit the script of the first character
    /// *with* a particular script. These heuristics may change in the future. For more, see UAX 24
    /// (<https://www.unicode.org/reports/tr24/>) Section 5.
    pub script: Script,
}

/// A span of text shaped with specific [`ShapeOptions`].
///
/// An [`Item`] represents a sequence of constant [`ShapeOptions`], but cannot always be passed to
/// the shaper as a single unit. Within an item, the script or bidirectional text embedding level
/// may change, which requires further splitting the item into segments of constant script and bidi
/// level (see [`Segment`]).
#[derive(Debug)]
pub struct Item<'a> {
    /// The character offset in the source text which this item ends.
    ///
    /// This must be strictly greater than the previous item's end. For the first item, it must be
    /// greater than 0.
    pub char_end: u32,

    /// The options to shape this item with.
    //
    // TODO: should users instead be allowed to build `options` at the `Segment` level (i.e.,
    // through some callback)? The motivation is for users to have access to the segment's `Script`
    // and be able to set font features based on that. If the entirety of `ShapeOptions` moves
    // there, it would allow users to set, e.g., a font size per segment, even though it's not
    // necessarily an item boundary; and note an item then doesn't mean a whole lot anymore. We
    // could also allow only some options to be per-segment. In any case, we're probably moving
    // towards a future where items reset grapheme segmentation, but segments do not (note Gecko and
    // Blink also reset grapheme segmentation when something like font size changes, but not when
    // the script changes).
    pub options: ShapeOptions<'a>,
    // TODO: we probably should allow users to pass in some data (like we allow passing
    // style_indices elsewhere), which we copy onto `ShapedRun`. That allows users to easily
    // correlate `ShapedRun`s with some data they themselves hold.
    //
    // This is potentially important for better correctness in `parley`: it itemizes based on
    // `nearly_eq` of shaping-relevant styles like font size, i.e., it should then read that item's
    // style to know the font size, even though it may be different for the run.
    // /// Opaque data copied onto every `ShapedRun` produced from this item.
    // pub user_data: u16,
}

/// Produces the items in a text via [`Self::next_until`]; created by [`Analysis::itemize`].
#[derive(Debug)]
pub(crate) struct Itemizer<'a> {
    /// The input text.
    text: &'a str,
    /// The per-char info, indexed by character index.
    char_info: &'a [CharInfo],
    /// The per-char bidi level, indexed by character index.
    bidi_levels: &'a [BidiLevel],

    /// The paragraph's base bidi level.
    ///
    /// If [`Self::bidi_levels`] is empty as a special case, this is the bidi level of each
    /// character.
    paragraph_bidi_level: BidiLevel,

    /// The running character offset of the last-processed item.
    current_char_offset: usize,
    /// The running script of the last-processed item.
    current_script: IcuScript,

    /// Character index of the next unconsumed character.
    pos: usize,
    /// Byte offset of the next unconsumed character.
    byte_pos: usize,
}

impl Analysis {
    /// Divide the `text` into individually-shapeable segments.
    ///
    /// The `text` passed in must be the same as used for producing the `self` analysis.
    ///
    /// The text is divided into items ending at the character offsets passed to
    /// [`Itemizer::next_until`], and further divided into segments of constant bidi level and
    /// script.
    ///
    /// Characters that don't have a particular script have their script resolved based on
    /// surrounding context (see [`Segment::script`]).
    pub(crate) fn itemize<'a>(&'a self, text: &'a str) -> Itemizer<'a> {
        let first_real_script = self
            .char_info()
            .iter()
            .map(|x| x.script)
            .find(|&script| real_script(script))
            .unwrap_or(IcuScript::Latin);

        Itemizer {
            text,
            char_info: self.char_info(),
            bidi_levels: self.bidi_levels(),
            paragraph_bidi_level: self.paragraph_level(),

            current_char_offset: 0,
            current_script: first_real_script,

            pos: 0,
            byte_pos: 0,
        }
    }
}

impl Itemizer<'_> {
    /// Produce the next segment, which must not extend past `char_end`.
    ///
    /// A segment ends at the first change in bidi level or script, or at `char_end`, whichever
    /// comes first. Callers pass the end of the item being shaped as `char_end`, so that items
    /// split the text in addition to bidi level and script.
    //
    // TODO: currently the items have the same effect as a change in bidi or script. This is not
    // how browsers handle things. In particular, items should reset grapheme segmentation, whereas
    // bidi and script should produce separately-shaped segments.
    pub(crate) fn next_until(&mut self, char_end: usize) -> Option<Segment> {
        let len = self.char_info.len();
        if self.pos >= len {
            debug_assert!(
                self.byte_pos == self.text.len()
                    && (self.bidi_levels.is_empty() || self.bidi_levels.len() == len),
                "`char_info`, `bidi_levels`, and `text` should now all be fully consumed \
                (though note `bidi_levels` may already have been empty as a special-case)"
            );
            return None;
        }
        debug_assert!(
            char_end > self.pos,
            "char_end must be greater than the running char offset"
        );

        let no_levels = self.bidi_levels.is_empty();
        let limit = char_end.min(len);
        let start_pos = self.pos;
        let mut i = start_pos;

        // First iteration: initialize item properties.
        let item_bidi_level = if no_levels {
            self.paragraph_bidi_level
        } else {
            self.bidi_levels[i]
        };
        {
            let mut script = self.char_info[i].script;
            if !real_script(script) {
                // This is a very simple heuristic, where if a character does not have a "real
                // script," it inherits the script of the preceding character. UAX 24 paragraph
                // 5.1 says this "works well in many cases", but also suggests performing, e.g.,
                // bracket matching (for example, the parentheses in `hello (αβγ)` should ideally
                // both be marked as being `Latin`). At that point, `Itemizer` would probably like
                // to have reusable scratch for the bracket stack.
                script = self.current_script;
            }
            self.current_script = script;
        }

        // Scan for the first bidi/script change, not passing `char_end`.
        i += 1;
        while i < limit {
            let bidi_level = if no_levels {
                self.paragraph_bidi_level
            } else {
                self.bidi_levels[i]
            };
            let mut script = self.char_info[i].script;
            if !real_script(script) {
                script = self.current_script;
            }
            if bidi_level != item_bidi_level || script != self.current_script {
                break;
            }
            i += 1;
        }

        // Advance the byte cursor past the consumed characters.
        let bytes = self.text.as_bytes();
        let mut byte_pos = self.byte_pos;
        for _ in 0..i - start_pos {
            byte_pos += match bytes[byte_pos] {
                ..0x80 => 1,
                0xC0..=0xDF => 2,
                0xE0..=0xEF => 3,
                _ => 4,
            };
        }

        let start_byte_offset = self.byte_pos;
        self.pos = i;
        self.byte_pos = byte_pos;
        let start_char_offset = self.current_char_offset;
        self.current_char_offset += i - start_pos;

        Some(Segment {
            range: TextRange {
                byte_range: start_byte_offset..byte_pos,
                char_range: start_char_offset..self.current_char_offset,
            },
            script: icu_script_to_parlance_script(self.current_script),
            bidi_level: item_bidi_level,
        })
    }
}

fn real_script(script: IcuScript) -> bool {
    script != IcuScript::Common && script != IcuScript::Unknown && script != IcuScript::Inherited
}

/// Convert an ICU script into a [`Script`].
#[inline]
fn icu_script_to_parlance_script(script: IcuScript) -> Script {
    static SHORT_NAMES: icu_properties::PropertyNamesShortBorrowed<'static, IcuScript> =
        icu_properties::PropertyNamesShort::new();

    SHORT_NAMES
        .get(script)
        .and_then(|name| Script::parse(name).ok())
        .unwrap_or(Script::UNKNOWN)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use parlance::Script;

    use crate::{Analysis, AnalysisOptions, Analyzer};

    use super::Segment;

    const LATN: Script = Script::from_bytes(*b"Latn");
    const GREK: Script = Script::from_bytes(*b"Grek");
    const ARAB: Script = Script::from_bytes(*b"Arab");

    fn analyze(text: &str) -> Analysis {
        let mut analyzer = Analyzer::new();
        let mut analysis = Analysis::new();
        let options = AnalysisOptions::default();
        analyzer.analyze(text, &options, &mut analysis);
        analysis
    }

    /// The segments of `text` as one single item, i.e. split only by bidi level and script.
    fn items(text: &str) -> Vec<Segment> {
        let analysis = analyze(text);
        let char_count = analysis.char_info().len();
        let mut itemizer = analysis.itemize(text);
        core::iter::from_fn(|| itemizer.next_until(char_count)).collect()
    }

    #[test]
    fn empty() {
        assert!(items("").is_empty());
    }

    #[test]
    fn mixed_direction() {
        let text = "hello مرحبا";
        let items = items(text);
        assert!(items.len() >= 2);
        assert_eq!(items[0].script, LATN);
        assert!(items[0].bidi_level.is_ltr());
        assert!(
            items
                .iter()
                .any(|item| item.script == ARAB && item.bidi_level.is_rtl())
        );

        // Items tile the text contiguously.
        let mut cursor = 0;
        for item in &items {
            assert_eq!(item.range.byte_range.start, cursor);
            cursor = item.range.byte_range.end;
        }
        assert_eq!(cursor, text.len());
    }

    #[test]
    fn item_ends_split_segments() {
        let text = "abcdef";
        let analysis = analyze(text);
        let mut itemizer = analysis.itemize(text);
        let items: Vec<_> = [3, 6]
            .into_iter()
            .map(|char_end| itemizer.next_until(char_end).unwrap())
            .collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].range.byte_range, 0..3);
        assert_eq!(items[0].range.char_range, 0..3);
        assert_eq!(items[1].range.byte_range, 3..6);
        assert_eq!(items[1].range.char_range, 3..6);
    }

    #[test]
    fn neutral_backward() {
        // Latin, a space (`Common`), then Greek.
        let text = "abc αβγ";
        let items = items(text);
        assert_eq!(items.len(), 2);
        // The space attaches to the Latin run.
        assert_eq!(&text[items[0].range.byte_range.clone()], "abc ");
        assert_eq!(items[0].script, LATN);
        assert_eq!(&text[items[1].range.byte_range.clone()], "αβγ");
        assert_eq!(items[1].script, GREK);
    }
}
