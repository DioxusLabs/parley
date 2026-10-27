// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Text analysis.
//!
//! Analysis is performed prior to shaping and is independent of fonts, turning a `&str` into
//! [`Analysis`].

use alloc::vec::Vec;
use core::ops::Range;

use cleft::{
    BidiClass, CoreProps, Dictionaries, GraphemeClusterBreak, GraphemeState, LineBreaks,
    LineOptions, LineStrictness, LineWordOption, RareProps, Utf8Scalars, WordBreaks, WordOptions,
};
use parlance::{BaseDirection, BidiLevel, LineBreak, Script, WordBreak};

use crate::bidi;
use crate::break_overrides::LineBreakContext;
use crate::{AnalysisOptions, Analyzer, LineBreakConfig};

/// The result of [`Analyzer::analyze`].
#[derive(Debug, Default)]
pub struct Analysis {
    /// Info for each character.
    pub(crate) info: Vec<CharInfo>,

    /// Bidi level for each character, parallel to `info`.
    ///
    /// Empty if the text is all LTR.
    pub(crate) levels: Vec<BidiLevel>,

    /// The base bidi level of the paragraph of text.
    pub(crate) paragraph_level: BidiLevel,
}

impl Analysis {
    /// Create a reusable [`Analysis`].
    ///
    /// Pass this to [`Analyzer::analyze`].
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the result while retaining capacity.
    pub(crate) fn clear(&mut self) {
        self.info.clear();
        self.levels.clear();
        self.paragraph_level = BidiLevel::new(0);
    }

    /// The per-character info in source order.
    #[inline(always)]
    pub fn char_info(&self) -> &[CharInfo] {
        &self.info
    }

    /// The bidi level for each character, parallel to [`Self::char_info`].
    ///
    /// Empty when the whole paragraph is left-to-right.
    #[inline(always)]
    pub fn bidi_levels(&self) -> &[BidiLevel] {
        &self.levels
    }

    /// The base bidi level of the paragraph of text.
    #[inline(always)]
    pub fn paragraph_level(&self) -> BidiLevel {
        self.paragraph_level
    }
}

/// Provides runtime data needed for text analysis.
///
/// Currently, this only includes optional word [`Dictionaries`] for the scripts whose line and
/// word breaks need one (Thai, Lao, Khmer, Myanmar, Chinese and Japanese). When none are given,
/// the dictionaries compiled in by the `complex-scripts` feature are used, if any.
#[derive(Default)]
pub struct AnalysisDataSources<'a> {
    dictionaries: Option<&'a Dictionaries>,
}

impl core::fmt::Debug for AnalysisDataSources<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnalysisDataSources")
            .field("external_dictionaries", &self.dictionaries.is_some())
            .finish()
    }
}

impl<'a> AnalysisDataSources<'a> {
    /// Create a new `AnalysisDataSources` which will use `dictionaries` for line and word
    /// segmentation. If not provided, the data compiled into the binary will be used instead (if
    /// present).
    pub const fn new(dictionaries: Option<&'a Dictionaries>) -> Self {
        Self { dictionaries }
    }
}

/// The dictionaries compiled in by the `complex-scripts` feature.
pub(crate) fn compiled_dictionaries() -> Dictionaries {
    #[cfg(feature = "complex-scripts")]
    {
        Dictionaries::with_compiled()
    }
    #[cfg(not(feature = "complex-scripts"))]
    {
        Dictionaries::new()
    }
}

/// The Unicode properties of one character that analysis reads: `cleft`'s two packed words plus
/// the few fixed sets `cleft` does not expose.
#[derive(Copy, Clone)]
pub(crate) struct Properties {
    ch: char,
    core: CoreProps,
    rare: RareProps,
}

impl Properties {
    #[inline(always)]
    pub(crate) fn get(ch: char) -> Self {
        Self {
            ch,
            core: cleft::core_props(ch),
            rare: cleft::rare_props(ch),
        }
    }

    #[inline(always)]
    pub(crate) fn script(self) -> Script {
        Script::from_bytes(self.core.script().tag())
    }

    #[inline(always)]
    pub(crate) fn grapheme_cluster_break(self) -> GraphemeClusterBreak {
        self.rare.grapheme_cluster_break()
    }

    #[inline(always)]
    pub(crate) fn bidi_class(self) -> BidiClass {
        self.core.bidi_class()
    }

    /// Whether the character has either of the `Emoji` or `Extended_Pictographic` properties.
    #[inline(always)]
    pub(crate) fn is_emoji_or_pictograph(self) -> bool {
        self.core.is_emoji() || self.core.is_extended_pictographic()
    }

    #[inline(always)]
    pub(crate) fn is_variation_selector(self) -> bool {
        is_variation_selector(self.ch)
    }

    #[inline(always)]
    pub(crate) fn is_regional_indicator(self) -> bool {
        self.core.is_regional_indicator()
    }

    /// UAX #14's mandatory breaks: `Line_Break` BK, CR, LF and NL.
    #[inline(always)]
    pub(crate) fn is_mandatory_linebreak(self) -> bool {
        self.core.is_paragraph_separator()
    }

    /// Whether the character is `Line_Break` SA (Thai, Lao, Khmer, Myanmar and the like), whose
    /// word boundaries need a dictionary.
    #[inline(always)]
    pub(crate) fn is_complex_context(self) -> bool {
        self.core.is_complex_context()
    }

    #[inline(always)]
    pub(crate) fn is_emoji(self) -> bool {
        self.core.is_emoji()
    }

    #[inline(always)]
    pub(crate) fn is_emoji_presentation(self) -> bool {
        self.core.is_emoji_presentation()
    }

    /// `Emoji_Modifier`: the five skin tone modifiers.
    #[inline(always)]
    pub(crate) fn is_emoji_modifier(self) -> bool {
        matches!(self.ch, '\u{1F3FB}'..='\u{1F3FF}')
    }

    #[inline(always)]
    pub(crate) fn is_emoji_modifier_base(self) -> bool {
        is_emoji_modifier_base(self.ch)
    }

    #[inline(always)]
    pub(crate) fn is_default_ignorable(self) -> bool {
        self.rare.is_default_ignorable()
    }

    /// `General_Category` `Cc`.
    #[inline(always)]
    pub(crate) fn is_control(self) -> bool {
        matches!(self.ch, '\0'..='\u{1F}' | '\u{7F}'..='\u{9F}')
    }

    /// Whether the character contributes glyphs to shaping. See [`contributes_to_shaping`].
    #[inline(always)]
    pub(crate) fn contributes_to_shaping(self) -> bool {
        if self.is_control() || matches!(self.ch, '\u{2028}' | '\u{2029}') {
            return false;
        }
        !(self.rare.is_default_ignorable()
            && !is_visible_default_ignorable(self.ch)
            && self.core.script() != cleft::ScriptId::INHERITED)
    }
}

/// `Variation_Selector`: the Mongolian free variation selectors, VS1 to VS16 and VS17 to VS256.
#[inline]
fn is_variation_selector(ch: char) -> bool {
    matches!(
        ch,
        '\u{180B}'..='\u{180D}' | '\u{180F}' | '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}'
    )
}

/// Default-ignorable code points that fonts nonetheless draw as spacing glyphs (`HarfBuzz` keeps
/// them visible): the Hangul fillers and the Duployan shorthand format controls.
#[inline]
fn is_visible_default_ignorable(ch: char) -> bool {
    matches!(
        ch,
        '\u{115F}' | '\u{1160}' | '\u{3164}' | '\u{FFA0}' | '\u{1BCA0}'..='\u{1BCA3}'
    )
}

/// `Emoji_Modifier_Base` as of Unicode 17 (`emoji-data.txt`).
fn is_emoji_modifier_base(ch: char) -> bool {
    matches!(
        ch,
        '\u{261D}'
            | '\u{26F9}'
            | '\u{270A}'..='\u{270D}'
            | '\u{1F385}'
            | '\u{1F3C2}'..='\u{1F3C4}'
            | '\u{1F3C7}'
            | '\u{1F3CA}'..='\u{1F3CC}'
            | '\u{1F442}'..='\u{1F443}'
            | '\u{1F446}'..='\u{1F450}'
            | '\u{1F466}'..='\u{1F478}'
            | '\u{1F47C}'
            | '\u{1F481}'..='\u{1F483}'
            | '\u{1F485}'..='\u{1F487}'
            | '\u{1F48F}'
            | '\u{1F491}'
            | '\u{1F4AA}'
            | '\u{1F574}'..='\u{1F575}'
            | '\u{1F57A}'
            | '\u{1F590}'
            | '\u{1F595}'..='\u{1F596}'
            | '\u{1F645}'..='\u{1F647}'
            | '\u{1F64B}'..='\u{1F64F}'
            | '\u{1F6A3}'
            | '\u{1F6B4}'..='\u{1F6B6}'
            | '\u{1F6C0}'
            | '\u{1F6CC}'
            | '\u{1F90C}'
            | '\u{1F90F}'
            | '\u{1F918}'..='\u{1F91F}'
            | '\u{1F926}'
            | '\u{1F930}'..='\u{1F939}'
            | '\u{1F93C}'..='\u{1F93E}'
            | '\u{1F977}'
            | '\u{1F9B5}'..='\u{1F9B6}'
            | '\u{1F9B8}'..='\u{1F9B9}'
            | '\u{1F9BB}'
            | '\u{1F9CD}'..='\u{1F9CF}'
            | '\u{1F9D1}'..='\u{1F9DD}'
            | '\u{1FAC3}'..='\u{1FAC5}'
            | '\u{1FAF0}'..='\u{1FAF8}'
    )
}

/// The line segmenter configuration for a [`LineBreakConfig`].
///
/// Configurations which result in the same segmentation map to the same key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SegmenterKey {
    word_option: LineWordOption,
    strictness: LineStrictness,
    /// See [`LineBreakConfig::language`]; the only languages which impact segmentation are
    /// Chinese and Japanese. These impact it in the same way, and only under
    /// [`LineBreak::Normal`] and [`LineBreak::Loose`], so we only track whether one of them
    /// applies.
    ja_zh: bool,
}

impl SegmenterKey {
    const DEFAULT: Self = Self {
        word_option: LineWordOption::Normal,
        strictness: LineStrictness::Strict,
        ja_zh: false,
    };

    fn new(config: &LineBreakConfig) -> Self {
        let strictness = match config.line_break {
            LineBreak::Strict => LineStrictness::Strict,
            LineBreak::Normal => LineStrictness::Normal,
            LineBreak::Loose => LineStrictness::Loose,
            // We add opportunities per grapheme ourselves.
            LineBreak::Anywhere => return Self::DEFAULT,
        };
        let word_option = match config.word_break {
            WordBreak::Normal => LineWordOption::Normal,
            WordBreak::BreakAll => LineWordOption::BreakAll,
            WordBreak::KeepAll => LineWordOption::KeepAll,
        };
        let ja_zh = match strictness {
            LineStrictness::Normal | LineStrictness::Loose => config
                .language
                .is_some_and(|language| matches!(language.language(), "ja" | "zh")),
            _ => false,
        };
        Self {
            word_option,
            strictness,
            ja_zh,
        }
    }

    fn line_options(self) -> LineOptions {
        LineOptions {
            strictness: self.strictness,
            word: self.word_option,
            ja_zh: self.ja_zh,
            cj_quotes: false,
            // CSS Text 3 allows `loose` to break before U+2010 and U+2013 after an ideograph in any
            // language (as ICU4X does); without this `cleft` follows ICU4C and only does so for
            // Chinese and Japanese.
            loose_hyphens: true,
        }
    }
}

/// Per-character analysis info.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CharInfo {
    /// The Unicode script this character belongs to.
    pub script: Script,
    flags: u16,
}

impl CharInfo {
    const VARIATION_SELECTOR_SHIFT: u16 = 0;
    const REGION_INDICATOR_SHIFT: u16 = 1;
    const CONTROL_SHIFT: u16 = 2;
    const EMOJI_OR_PICTOGRAPH_SHIFT: u16 = 3;
    const CONTRIBUTES_TO_SHAPING_SHIFT: u16 = 4;
    const FORCE_NORMALIZE_SHIFT: u16 = 5;
    const GRAPHEME_START_SHIFT: u16 = 6;
    const WORD_BOUNDARY_SHIFT: u8 = 7;
    const SOFT_WRAP_OPPORTUNITY_SHIFT: u16 = 8;
    const EMOJI_SHIFT: u16 = 9;
    const EMOJI_PRESENTATION_SHIFT: u16 = 10;
    const EMOJI_MODIFIER_SHIFT: u16 = 11;
    const EMOJI_MODIFIER_BASE_SHIFT: u16 = 12;
    const INLINE_OBJECT_SHIFT: u16 = 13;
    const DEFAULT_IGNORABLE_SHIFT: u16 = 14;

    const VARIATION_SELECTOR_MASK: u16 = 1 << Self::VARIATION_SELECTOR_SHIFT;
    const REGION_INDICATOR_MASK: u16 = 1 << Self::REGION_INDICATOR_SHIFT;
    const CONTROL_MASK: u16 = 1 << Self::CONTROL_SHIFT;
    const EMOJI_OR_PICTOGRAPH_MASK: u16 = 1 << Self::EMOJI_OR_PICTOGRAPH_SHIFT;
    const CONTRIBUTES_TO_SHAPING_MASK: u16 = 1 << Self::CONTRIBUTES_TO_SHAPING_SHIFT;
    const FORCE_NORMALIZE_MASK: u16 = 1 << Self::FORCE_NORMALIZE_SHIFT;
    const GRAPHEME_START_MASK: u16 = 1 << Self::GRAPHEME_START_SHIFT;
    const WORD_BOUNDARY_MASK: u16 = 1 << Self::WORD_BOUNDARY_SHIFT;
    const SOFT_WRAP_OPPORTUNITY_MASK: u16 = 1 << Self::SOFT_WRAP_OPPORTUNITY_SHIFT;
    const EMOJI_MASK: u16 = 1 << Self::EMOJI_SHIFT;
    const EMOJI_PRESENTATION_MASK: u16 = 1 << Self::EMOJI_PRESENTATION_SHIFT;
    const EMOJI_MODIFIER_MASK: u16 = 1 << Self::EMOJI_MODIFIER_SHIFT;
    const EMOJI_MODIFIER_BASE_MASK: u16 = 1 << Self::EMOJI_MODIFIER_BASE_SHIFT;
    const INLINE_OBJECT_MASK: u16 = 1 << Self::INLINE_OBJECT_SHIFT;
    const DEFAULT_IGNORABLE_MASK: u16 = 1 << Self::DEFAULT_IGNORABLE_SHIFT;

    fn new(
        script: Script,
        is_variation_selector: bool,
        is_region_indicator: bool,
        is_control: bool,
        is_emoji_or_pictograph: bool,
        contributes_to_shaping: bool,
        force_normalize: bool,
        is_grapheme_start: bool,
        is_word_boundary: bool,
        is_soft_wrap_opportunity: bool,
        is_emoji: bool,
        is_emoji_presentation: bool,
        is_emoji_modifier: bool,
        is_emoji_modifier_base: bool,
        is_default_ignorable: bool,
    ) -> Self {
        Self {
            script,
            flags: (is_variation_selector as u16) << Self::VARIATION_SELECTOR_SHIFT
                | (is_region_indicator as u16) << Self::REGION_INDICATOR_SHIFT
                | (is_control as u16) << Self::CONTROL_SHIFT
                | (is_emoji_or_pictograph as u16) << Self::EMOJI_OR_PICTOGRAPH_SHIFT
                | (contributes_to_shaping as u16) << Self::CONTRIBUTES_TO_SHAPING_SHIFT
                | (force_normalize as u16) << Self::FORCE_NORMALIZE_SHIFT
                | (is_grapheme_start as u16) << Self::GRAPHEME_START_SHIFT
                | (is_word_boundary as u16) << Self::WORD_BOUNDARY_SHIFT
                | (is_soft_wrap_opportunity as u16) << Self::SOFT_WRAP_OPPORTUNITY_SHIFT
                | (is_emoji as u16) << Self::EMOJI_SHIFT
                | (is_emoji_presentation as u16) << Self::EMOJI_PRESENTATION_SHIFT
                | (is_emoji_modifier as u16) << Self::EMOJI_MODIFIER_SHIFT
                | (is_emoji_modifier_base as u16) << Self::EMOJI_MODIFIER_BASE_SHIFT
                | (is_default_ignorable as u16) << Self::DEFAULT_IGNORABLE_SHIFT,
        }
    }

    /// Whether this character is a variation selector.
    #[inline(always)]
    pub fn is_variation_selector(self) -> bool {
        self.flags & Self::VARIATION_SELECTOR_MASK != 0
    }

    /// Whether this character is a regional indicator symbol.
    #[inline(always)]
    pub fn is_region_indicator(self) -> bool {
        self.flags & Self::REGION_INDICATOR_MASK != 0
    }

    /// Whether this character is a control character.
    #[inline(always)]
    pub fn is_control(self) -> bool {
        self.flags & Self::CONTROL_MASK != 0
    }

    /// Returns whether the character has either of the `Emoji` or`Extended_Pictographic` properties.
    ///
    /// Parley Engine uses this to determine whether a character might be part of an Emoji, and so whether to do
    /// more stringent checks.
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji_or_pictograph(self) -> bool {
        self.flags & Self::EMOJI_OR_PICTOGRAPH_MASK != 0
    }

    /// Whether this character has the `Emoji` property ([UTS #51][]).
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji(self) -> bool {
        self.flags & Self::EMOJI_MASK != 0
    }

    /// Whether this character has the `Emoji_Presentation` property ([UTS #51][]), i.e. it is
    /// displayed as an emoji by default.
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji_presentation(self) -> bool {
        self.flags & Self::EMOJI_PRESENTATION_MASK != 0
    }

    /// Whether this character has the `Emoji_Modifier` property ([UTS #51][]), i.e. it is a
    /// skin tone modifier.
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji_modifier(self) -> bool {
        self.flags & Self::EMOJI_MODIFIER_MASK != 0
    }

    /// Whether this character has the `Emoji_Modifier_Base` property ([UTS #51][]), i.e. it can
    /// be followed by a skin tone modifier.
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji_modifier_base(self) -> bool {
        self.flags & Self::EMOJI_MODIFIER_BASE_MASK != 0
    }

    /// Whether this character has the [`Default_Ignorable_Code_Point`] property.
    ///
    /// This property is used in Parley to disable letter-spacing for some atoms, matching Blink.
    ///
    /// [`Default_Ignorable_Code_Point`]: https://www.unicode.org/reports/tr44/#Default_Ignorable_Code_Point
    #[inline(always)]
    pub fn is_default_ignorable(self) -> bool {
        self.flags & Self::DEFAULT_IGNORABLE_MASK != 0
    }

    /// Whether this character contributes glyphs to shaping (`false` for control characters and
    /// most format characters).
    #[inline(always)]
    pub fn contributes_to_shaping(self) -> bool {
        self.flags & Self::CONTRIBUTES_TO_SHAPING_MASK != 0
    }

    /// Whether this character should be normalized before glyph mapping during shaping.
    #[inline(always)]
    pub fn force_normalize(self) -> bool {
        self.flags & Self::FORCE_NORMALIZE_MASK != 0
    }

    /// Whether this character begins a grapheme cluster ([UAX #29 § 3][graphemes]).
    ///
    /// [graphemes]: https://www.unicode.org/reports/tr29/#Grapheme_Cluster_Boundaries
    #[inline(always)]
    pub fn is_grapheme_start(self) -> bool {
        self.flags & Self::GRAPHEME_START_MASK != 0
    }

    /// Whether there is a word boundary before this character ([UAX #29 § 4][words]).
    ///
    /// [words]: https://www.unicode.org/reports/tr29/#Word_Boundaries
    #[inline(always)]
    pub fn is_word_boundary(self) -> bool {
        self.flags & Self::WORD_BOUNDARY_MASK != 0
    }

    /// Whether there is a soft wrap opportunity before this character ([UAX #14][line-breaking]).
    ///
    /// Note mandatory breaks (like `\n`) are encoded as [`Whitespace`](crate::shape::Whitespace).
    ///
    /// [line-breaking]: https://www.unicode.org/reports/tr14/
    #[inline(always)]
    pub fn is_soft_wrap_opportunity(self) -> bool {
        self.flags & Self::SOFT_WRAP_OPPORTUNITY_MASK != 0
    }

    /// Whether this character stands in for an inline object.
    ///
    /// See [`AnalysisOptions::inline_objects`].
    #[inline(always)]
    pub fn is_inline_object(self) -> bool {
        self.flags & Self::INLINE_OBJECT_MASK != 0
    }
}

pub(crate) fn analyze_text(
    analyzer: &mut Analyzer,
    text: &str,
    options: &AnalysisOptions<'_>,
    analysis: &mut Analysis,
    data_sources: &AnalysisDataSources<'_>,
) {
    /// Turns the sparse, sorted, non-overlapping `options.line_break` into a contiguous sequence of
    /// `(range, segmenter key)` segments covering all of `text`.
    ///
    /// Any region not covered by an override takes the default configuration.
    struct DenseLineBreaks<'a> {
        line_break: &'a [(Range<usize>, LineBreakConfig)],
        /// Index of the next word break to emit.
        next: usize,
        /// Start of the next segment to emit.
        cursor: usize,
        text_len: usize,
    }

    impl<'a> DenseLineBreaks<'a> {
        fn new(line_break: &'a [(Range<usize>, LineBreakConfig)], text_len: usize) -> Self {
            Self {
                line_break,
                next: 0,
                cursor: 0,
                text_len,
            }
        }
    }

    impl Iterator for DenseLineBreaks<'_> {
        type Item = (Range<usize>, SegmenterKey);

        fn next(&mut self) -> Option<Self::Item> {
            if self.cursor >= self.text_len {
                return None;
            }

            // Ignore empty ranges.
            while self
                .line_break
                .get(self.next)
                .is_some_and(|(range, _)| range.is_empty())
            {
                self.next += 1;
            }

            match self.line_break.get(self.next) {
                // A gap before the next override: fill it with the default up to its start.
                Some((range, _)) if self.cursor < range.start => {
                    let segment = self.cursor..range.start;
                    self.cursor = range.start;
                    Some((segment, SegmenterKey::DEFAULT))
                }
                // At the next override: emit it.
                Some((range, config)) => {
                    self.cursor = range.end;
                    self.next += 1;
                    Some((range.start..range.end, SegmenterKey::new(config)))
                }
                // No overrides remain: fill the default to the end.
                None => {
                    let segment = self.cursor..self.text_len;
                    self.cursor = self.text_len;
                    Some((segment, SegmenterKey::DEFAULT))
                }
            }
        }
    }

    if text.is_empty() {
        analyzer
            .bidi
            .resolve(core::iter::empty(), options.base_direction);
        analysis.paragraph_level = analyzer.bidi.base_level();
        return;
    }

    let dictionaries = data_sources.dictionaries.unwrap_or(&analyzer.dictionaries);

    // Line boundaries (word break naming refers to the line boundary determination config).
    //
    // The text is split into runs of equal line boundary config (part of style information). Each
    // run is scanned under its own options; a run after the first starts its scan at the last
    // opportunity before it (reading the text before that for context), and keeps only the
    // opportunities from its start on, so the new options decide the pair at the seam.
    let line_boundary_positions = &mut analyzer.line_boundary_positions;
    line_boundary_positions.clear();
    for (range, key) in DenseLineBreaks::new(options.line_break, text.len()) {
        let opts = key.line_options();
        if range.start == 0 {
            line_boundary_positions.extend(
                LineBreaks::new(Utf8Scalars::new(&text[..range.end]), opts)
                    .with_dictionaries(dictionaries),
            );
            continue;
        }
        let resume = line_boundary_positions.last().copied().unwrap_or(0);
        let breaks = LineBreaks::with_preceding(
            Utf8Scalars::with_start(&text[..range.end], resume),
            opts,
            text[..resume].chars().rev(),
        )
        .with_dictionaries(dictionaries);
        line_boundary_positions.extend(breaks.filter(|&b| b >= range.start));
    }

    // Collect boundary byte positions compactly
    let mut wb_iter = WordBreaks::new(Utf8Scalars::new(text), WordOptions::new())
        .with_dictionaries(dictionaries)
        .peekable();
    let mut grapheme_state = GraphemeState::new();

    // Merge boundaries - line takes precedence over word
    let mut lb_iter = line_boundary_positions.iter().peekable();
    let mut prev_char = None;
    let mut prev_prev_char = None;
    // Whether the preceding character is a mandatory break.
    let mut prev_is_mandatory_linebreak = false;
    // Whether the preceding character is a break-space, i.e. a space, tab or ideographic space in
    // a `break_spaces` range.
    let mut prev_is_break_space = false;
    let mut break_spaces_iter = options.break_spaces.iter().peekable();
    let mut break_anywhere_iter = options
        .line_break
        .iter()
        .filter(|(_, config)| config.line_break == LineBreak::Anywhere)
        .map(|(range, _)| range)
        .peekable();
    let boundary_iter = text.char_indices().map(|(byte_pos, ch)| {
        // advance any stale word boundary positions
        while let Some(&w) = wb_iter.peek() {
            if w < byte_pos {
                _ = wb_iter.next();
            } else {
                break;
            }
        }
        // advance any stale line boundary positions
        while let Some(&l) = lb_iter.peek() {
            if *l < byte_pos {
                _ = lb_iter.next();
            } else {
                break;
            }
        }

        // The start of the text is a word boundary (WB1); `WordBreaks` yields only the inner ones.
        let mut is_word = byte_pos == 0;
        if let Some(&w) = wb_iter.peek()
            && w == byte_pos
        {
            is_word = true;
            _ = wb_iter.next();
        }
        let properties = Properties::get(ch);
        let is_grapheme_start = grapheme_state.advance(properties.rare.grapheme_props());

        // A run of a Line_Break SA script with no dictionary is one word to `cleft`; apply
        // UAX #29 Rule WB999 instead, a boundary at every grapheme cluster.
        if !is_word
            && is_grapheme_start
            && properties.is_complex_context()
            && !dictionaries.segments(ch)
        {
            is_word = true;
        }

        let is_line = if let Some(&l) = lb_iter.peek()
            && *l == byte_pos
        {
            _ = lb_iter.next();
            true
        } else {
            false
        };

        // CSS Text 4 § 4.3.1 `break-spaces`: a soft wrap opportunity exists after each preserved
        // space or tab (but never directly before a mandatory break, UAX #14 LB6).
        while break_spaces_iter
            .peek()
            .is_some_and(|range| range.end <= byte_pos)
        {
            _ = break_spaces_iter.next();
        }
        let after_break_space = prev_is_break_space;
        prev_is_break_space = break_spaces_iter
            .peek()
            .is_some_and(|range| range.contains(&byte_pos))
            && matches!(ch, ' ' | '\t' | '\u{3000}');

        while break_anywhere_iter
            .peek()
            .is_some_and(|range| range.end <= byte_pos)
        {
            _ = break_anywhere_iter.next();
        }
        let is_break_anywhere = break_anywhere_iter
            .peek()
            .is_some_and(|range| range.contains(&byte_pos));

        let is_mandatory_linebreak = properties.is_mandatory_linebreak();

        // We never have a soft wrap opportunity before a mandatory break (following UAX #14 LB6),
        // and we never have one right after a mandatory break: the mandatory break is handled
        // separately.
        let is_soft_wrap_opportunity = if is_mandatory_linebreak || prev_is_mandatory_linebreak {
            false
        } else {
            let mut opportunity = if is_break_anywhere {
                // `line-break: anywhere`: an opportunity around every grapheme.
                prev_char.is_some() && is_grapheme_start
            } else {
                is_line || after_break_space
            };
            // Consumers can override soft wrap opportunities, except around a mandatory
            // break.
            if let (Some(prev), Some(lb_override)) = (prev_char, options.line_break_override) {
                let forced = lb_override(LineBreakContext {
                    before_before: prev_prev_char,
                    before: prev,
                    after: ch,
                });
                if let Some(forced) = forced {
                    opportunity = forced;
                }
            }
            opportunity
        };

        prev_prev_char = prev_char;
        prev_char = Some(ch);
        prev_is_mandatory_linebreak = is_mandatory_linebreak;

        (
            is_soft_wrap_opportunity,
            is_word,
            is_grapheme_start,
            ch,
            properties,
        )
    });

    let mut needs_bidi_resolution = false;

    analysis.info.reserve(text.len());
    for (is_soft_wrap_opportunity, is_word, is_grapheme_start, ch, properties) in boundary_iter {
        let script = properties.script();
        let grapheme_cluster_break = properties.grapheme_cluster_break();
        let bidi_class = properties.bidi_class();
        let is_emoji_or_pictograph = properties.is_emoji_or_pictograph();
        let is_variation_selector = properties.is_variation_selector();
        let is_region_indicator = properties.is_regional_indicator();

        let force_normalize = {
            // "Extend" break chars should be normalized first, with two exceptions
            if matches!(grapheme_cluster_break, GraphemeClusterBreak::Extend) &&
                    ch as u32 != 0x200C && // Is not a Zero Width Non-Joiner &&
                    !is_variation_selector
            {
                true
            } else {
                // All spacing mark break chars should be normalized first.
                matches!(grapheme_cluster_break, GraphemeClusterBreak::SpacingMark)
            }
        };

        needs_bidi_resolution |= bidi::needs_bidi_resolution(bidi_class);

        analysis.info.push(CharInfo::new(
            script,
            is_variation_selector,
            is_region_indicator,
            properties.is_control(),
            is_emoji_or_pictograph,
            properties.contributes_to_shaping(),
            force_normalize,
            is_grapheme_start,
            is_word,
            is_soft_wrap_opportunity,
            properties.is_emoji(),
            properties.is_emoji_presentation(),
            properties.is_emoji_modifier(),
            properties.is_emoji_modifier_base(),
            properties.is_default_ignorable(),
        ));
    }

    if !options.inline_objects.is_empty() {
        mark_inline_objects(text, options, &mut analysis.info);
    }

    if needs_bidi_resolution || options.base_direction == BaseDirection::Rtl {
        analyzer.bidi.resolve(
            text.chars().map(|ch| {
                let core = cleft::core_props(ch);
                let bracket = core
                    .closing_bracket()
                    .map(|closing| (closing, core.opens_bracket()));
                (core.bidi_class(), bracket)
            }),
            options.base_direction,
        );
        core::mem::swap(&mut analysis.levels, &mut analyzer.bidi.levels);
        analysis.paragraph_level = analyzer.bidi.base_level();
    }
}

/// Mark the inline objects of `text` in `info` (see [`AnalysisOptions::inline_objects`]).
///
/// We assume inline objects are rare relative to normal text, so we make the choice to patch the
/// analysis of each inline object and of the character after it.
///
/// For the same reason this is marked `#[inline(never)]`: inflating the caller's body will probably
/// cost more on average than inlining would win.
#[inline(never)]
fn mark_inline_objects(text: &str, options: &AnalysisOptions<'_>, info: &mut [CharInfo]) {
    const LEN: usize = '\u{FFFC}'.len_utf8();
    let is_mandatory = |ch| Properties::get(ch).is_mandatory_linebreak();

    // Between two objects, suppressing an opportunity wins.
    let combine = |a: Option<bool>, b: Option<bool>| match (a, b) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        _ => a.or(b),
    };

    let mut char_index = 0;
    let mut counted_to = 0;
    for (i, object) in options.inline_objects.iter().enumerate() {
        let byte = object.text_byte_start;
        debug_assert!(
            text[byte..].starts_with('\u{FFFC}'),
            "An inline object must be a U+FFFC OBJECT REPLACEMENT CHARACTER, found none at byte {byte}"
        );
        char_index += text[counted_to..byte].chars().count();
        counted_to = byte;
        info[char_index].flags = info[char_index].flags & !CharInfo::CONTRIBUTES_TO_SHAPING_MASK
            | CharInfo::INLINE_OBJECT_MASK;

        let break_before = match options.inline_objects[..i].last() {
            Some(previous) if previous.text_byte_start + LEN == byte => {
                combine(previous.break_after, object.break_before)
            }
            _ => object.break_before,
        };
        let break_after = match options.inline_objects.get(i + 1) {
            Some(next) if byte + LEN == next.text_byte_start => {
                combine(object.break_after, next.break_before)
            }
            _ => object.break_after,
        };

        // The object is a grapheme of its own, with soft wrap opportunities specified by its break
        // conditions.
        let mut before = text[..byte].chars().rev();
        let (before, before_before) = (before.next(), before.next());
        let after = text[byte + LEN..].chars().next();
        for (char_index, byte, [before_before, before, after], condition) in [
            (
                char_index,
                byte,
                [before_before, before, Some('\u{FFFC}')],
                break_before,
            ),
            (
                char_index + 1,
                byte + LEN,
                [before, Some('\u{FFFC}'), after],
                break_after,
            ),
        ] {
            let Some(info) = info.get_mut(char_index) else {
                continue;
            };
            info.flags |= CharInfo::GRAPHEME_START_MASK;
            let (Some(before), Some(after)) = (before, after) else {
                continue;
            };
            if is_mandatory(before) || is_mandatory(after) {
                continue;
            }
            let forced = options.line_break_override.and_then(|lb_override| {
                lb_override(LineBreakContext {
                    before_before,
                    before,
                    after,
                })
            });
            let is_break_anywhere = {
                let i = options
                    .line_break
                    .partition_point(|(range, _)| range.end <= byte);
                options.line_break.get(i).is_some_and(|(range, config)| {
                    range.contains(&byte) && config.line_break == LineBreak::Anywhere
                })
            };
            // Overrides take precedence, then the object's conditions, then the text's rules.
            let opportunity = forced
                .or(condition)
                .unwrap_or_else(|| info.is_soft_wrap_opportunity() || is_break_anywhere);
            info.flags = info.flags & !CharInfo::SOFT_WRAP_OPPORTUNITY_MASK
                | (opportunity as u16) << CharInfo::SOFT_WRAP_OPPORTUNITY_SHIFT;
        }
    }
}

/// All characters contribute to shaping except:
/// - Control characters (`General_Category` `Cc`) and the line and paragraph separators
/// - Default-ignorable code points that fonts do not draw, unless they use the "Inherited"
///   script (joiners and variation selectors, which the shaper reads)
#[inline(always)]
pub(crate) fn contributes_to_shaping(ch: char) -> bool {
    Properties::get(ch).contributes_to_shaping()
}
