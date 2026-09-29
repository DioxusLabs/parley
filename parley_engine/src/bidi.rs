// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Unicode bidirectional algorithm.

use alloc::vec::Vec;
use icu_properties::props::{BidiClass, BidiMirroringGlyph, BidiPairedBracketType};
use parlance::{BaseDirection, BidiLevel};

/// Resolver for the Unicode bidirectional algorithm.
#[derive(Clone, Default)]
pub struct BidiResolver {
    base_level: BidiLevel,
    pub(crate) levels: Vec<BidiLevel>,
    initial_types: Vec<BidiClass>,
    types: Vec<BidiClass>,
    brackets: Vec<(usize, char, BidiMirroringGlyph)>,
    bracket_pairs: Vec<(usize, usize)>,
    runs: Vec<Run>,
    indices: Vec<usize>,
    /// Character index of the first character of each unit, parallel to `initial_types`.
    unit_starts: Vec<usize>,
    flags: u16,
}

impl core::fmt::Debug for BidiResolver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BidiResolver")
            .field("base_level", &self.base_level)
            .field("levels", &self.levels)
            .finish_non_exhaustive()
    }
}

impl BidiResolver {
    /// Creates a new resolver.
    pub fn new() -> Self {
        Self {
            base_level: BidiLevel::new(0),
            levels: Vec::new(),
            initial_types: Vec::new(),
            types: Vec::new(),
            brackets: Vec::new(),
            bracket_pairs: Vec::new(),
            runs: Vec::new(),
            indices: Vec::new(),
            unit_starts: Vec::new(),
            flags: 0,
        }
    }

    /// Returns the base level of the text.
    pub fn base_level(&self) -> BidiLevel {
        self.base_level
    }

    /// Returns the sequence of bidi levels corresponding to all characters in the
    /// paragraph.
    pub fn levels(&self) -> &[BidiLevel] {
        &self.levels
    }

    /// Clears the resolver state.
    pub fn clear(&mut self) {
        self.initial_types.clear();
        self.unit_starts.clear();
        self.levels.clear();
        self.types.clear();
        self.brackets.clear();
        self.bracket_pairs.clear();
        self.flags = 0;
        self.base_level = BidiLevel::new(0);
    }

    /// Resolves a paragraph with the specified base direction and
    /// precomputed types.
    pub fn resolve(
        &mut self,
        chars: impl Iterator<Item = (char, (BidiClass, BidiMirroringGlyph))>,
        base_direction: BaseDirection,
    ) {
        self.resolve_impl(chars, base_direction, true);
    }

    /// Resolves the paragraph over "units" rather than characters.
    ///
    /// When `merge` is set, a strong character absorbs the following characters that are
    /// guaranteed to resolve to the same level as it, up to and including the next strong
    /// character of the same class (see [`merge_mask`]). Such a span behaves exactly like the
    /// single strong character in every rule of the algorithm, so resolving it as one unit
    /// produces identical levels while doing far less work on long runs of same-direction text.
    fn resolve_impl(
        &mut self,
        chars: impl Iterator<Item = (char, (BidiClass, BidiMirroringGlyph))>,
        base_direction: BaseDirection,
        merge: bool,
    ) {
        self.clear();
        let mut needs_bidi = false;
        let mut len = 0;
        // The unit index and class of the last strong unit that can still be extended.
        let mut merge_target: Option<(usize, BidiClass)> = None;
        for (ch, (t, bracket)) in chars {
            let i = len;
            len += 1;
            needs_bidi = needs_bidi || mask(t) & BIDI_MASK != 0;
            let is_bracket = bracket.paired_bracket_type != BidiPairedBracketType::None;

            if let Some((unit, class)) = merge_target {
                if !is_bracket && mask(t) & merge_mask(class) != 0 {
                    if t == class {
                        // Absorb this character and any pending units since `unit`.
                        self.initial_types.truncate(unit + 1);
                        self.unit_starts.truncate(unit + 1);
                    } else {
                        // Pending: absorbed only if another `class` character follows.
                        self.initial_types.push(t);
                        self.unit_starts.push(i);
                    }
                    continue;
                }
                merge_target = None;
            }

            if is_bracket {
                self.brackets.push((self.initial_types.len(), ch, bracket));
            }
            if merge && mask(t) & STRONG_MASK != 0 {
                merge_target = Some((self.initial_types.len(), t));
            }
            self.initial_types.push(t);
            self.unit_starts.push(i);
        }
        self.base_level = match base_direction {
            BaseDirection::Auto => Self::default_level(&self.initial_types),
            BaseDirection::Ltr => BidiLevel::new(0),
            BaseDirection::Rtl => BidiLevel::new(1),
        };
        if !needs_bidi && self.base_level == BidiLevel::new(0) {
            self.flags |= 1;
            self.levels.resize(len, self.base_level);
            return;
        }
        self.resolve_units();
        let units = self.initial_types.len();
        if units != len {
            // Expand unit levels to character levels, back to front so that each unit's level
            // is read before it can be overwritten (`unit_starts[u] >= u`).
            self.levels.resize(len, BidiLevel::new(0));
            let mut end = len;
            for u in (0..units).rev() {
                let start = self.unit_starts[u];
                let level = self.levels[u];
                self.levels[start..end].fill(level);
                end = start;
            }
        }
    }

    fn resolve_units(&mut self) {
        let len = self.initial_types.len();
        self.types.extend_from_slice(&self.initial_types);
        self.resolve_levels();
        self.resolve_runs();
        //self.dump_sequences();
        for i in 0..self.runs.len() {
            if self.runs[i].in_sequence {
                continue;
            }
            self.types.truncate(len);
            self.indices.clear();
            let mut cur = i;
            let level = self.runs[i].level;
            let sos = self.runs[i].sos;
            let mut eos;
            loop {
                let run = &self.runs[cur];
                for i in run.start..run.end {
                    let ty = self.types[i];
                    if !is_removed_by_x9(ty) {
                        self.types.push(ty);
                        self.indices.push(i);
                    }
                }
                eos = run.eos;
                cur = match run.next {
                    Some(i) => i,
                    None => break,
                };
            }
            self.resolve_sequence(level.to_u8(), sos, eos, self.indices.len());
        }
        for i in 0..len {
            let t = self.initial_types[i];
            if t == BidiClass::SegmentSeparator || t == BidiClass::ParagraphSeparator {
                self.levels[i] = self.base_level;
                for j in (0..i).rev() {
                    let t = self.initial_types[j];
                    if is_removed_by_x9(t) {
                        continue;
                    } else if t == BidiClass::WhiteSpace
                        || is_isolate_initiator(t)
                        || t == BidiClass::PopDirectionalIsolate
                    {
                        self.levels[j] = self.base_level;
                    } else {
                        break;
                    }
                }
            } else if is_removed_by_x9(t) {
                if i == 0 {
                    self.levels[i] = self.base_level;
                } else {
                    self.levels[i] = self.levels[i - 1];
                }
                //self.levels[i] = 0xFF;
            }
        }
        for i in (0..len).rev() {
            let t = self.initial_types[i];
            if is_removed_by_x9(t) {
                continue;
            } else if t == BidiClass::WhiteSpace
                || is_isolate_initiator(t)
                || t == BidiClass::PopDirectionalIsolate
            {
                //self.levels[i] = self.base_level;
            } else {
                break;
            }
        }
    }

    fn default_level(types: &[BidiClass]) -> BidiLevel {
        let mut isolates = 0;
        for ty in types {
            let ty = *ty;
            match ty {
                BidiClass::RightToLeftIsolate
                | BidiClass::LeftToRightIsolate
                | BidiClass::FirstStrongIsolate => isolates += 1,
                BidiClass::PopDirectionalIsolate if isolates > 0 => isolates -= 1,
                BidiClass::LeftToRight | BidiClass::RightToLeft | BidiClass::ArabicLetter
                    if isolates == 0 =>
                {
                    return if ty == BidiClass::LeftToRight {
                        BidiLevel::new(0)
                    } else {
                        BidiLevel::new(1)
                    };
                }
                _ => {}
            }
        }
        BidiLevel::new(0)
    }

    fn default_level_until_pdi(types: &[BidiClass]) -> u8 {
        let mut isolates = 0;
        for ty in types {
            let ty = *ty;
            match ty {
                BidiClass::RightToLeftIsolate
                | BidiClass::LeftToRightIsolate
                | BidiClass::FirstStrongIsolate => isolates += 1,
                BidiClass::PopDirectionalIsolate => {
                    if isolates > 0 {
                        isolates -= 1;
                    } else {
                        return 0;
                    }
                }
                BidiClass::LeftToRight | BidiClass::RightToLeft | BidiClass::ArabicLetter
                    if isolates == 0 =>
                {
                    return if ty == BidiClass::LeftToRight { 0 } else { 1 };
                }
                _ => {}
            }
        }
        0
    }

    fn resolve_levels(&mut self) {
        let base = self.base_level;
        let len = self.types.len();
        self.levels.clear();
        self.levels.resize(len, BidiLevel::new(0));
        let mut stack = Stack::new();
        let mut overflow_isolates = 0;
        let mut overflow_embedding = 0;
        let mut valid_isolates = 0;
        stack.push(base, BidiClass::OtherNeutral, false);
        for i in 0..len {
            let t = self.types[i];
            let tmask = mask(t);
            if tmask & EXPLICIT_MASK != 0 {
                let is_isolate = tmask & ISOLATE_MASK != 0;
                let is_rtl = if t == BidiClass::FirstStrongIsolate && i + 1 < len {
                    Self::default_level_until_pdi(&self.types[i + 1..]) == 1
                } else {
                    tmask & RTL_MASK != 0
                };
                if is_isolate {
                    self.levels[i] = stack.embedding_level();
                    let os = stack.override_status();
                    if os != BidiClass::OtherNeutral {
                        self.types[i] = os;
                    }
                }
                let new_level = if is_rtl {
                    stack.embedding_level().next_odd()
                } else {
                    stack.embedding_level().next_even()
                };
                if new_level <= BidiLevel::MAX && overflow_isolates == 0 && overflow_embedding == 0
                {
                    if is_isolate {
                        valid_isolates += 1;
                    }
                    stack.push(
                        new_level,
                        if t == BidiClass::LeftToRightOverride {
                            BidiClass::LeftToRight
                        } else if t == BidiClass::RightToLeftOverride {
                            BidiClass::RightToLeft
                        } else {
                            BidiClass::OtherNeutral
                        },
                        is_isolate,
                    );
                } else if is_isolate {
                    overflow_isolates += 1;
                } else if overflow_isolates == 0 {
                    overflow_embedding += 1;
                }
            } else if t == BidiClass::PopDirectionalIsolate {
                if overflow_isolates > 0 {
                    overflow_isolates -= 1;
                } else if valid_isolates == 0 {
                    // empty
                } else {
                    overflow_embedding = 0;
                    while !stack.isolate_status() {
                        stack.pop();
                    }
                    stack.pop();
                    valid_isolates -= 1;
                }
                self.levels[i] = stack.embedding_level();
                if stack.override_status() != BidiClass::OtherNeutral {
                    self.types[i] = stack.override_status();
                }
            } else if t == BidiClass::PopDirectionalFormat {
                self.levels[i] = stack.embedding_level();
                if overflow_isolates > 0 {
                    // empty
                } else if overflow_embedding > 0 {
                    overflow_embedding -= 1;
                } else if !stack.isolate_status() && stack.depth >= 2 {
                    stack.pop();
                }
            } else if t == BidiClass::ParagraphSeparator {
                stack.depth = 1;
                overflow_isolates = 0;
                overflow_embedding = 0;
                valid_isolates = 0;
                self.levels[i] = base;
            } else if t != BidiClass::BoundaryNeutral {
                self.levels[i] = stack.embedding_level();
                if stack.override_status() != BidiClass::OtherNeutral {
                    self.types[i] = stack.override_status();
                }
            }
        }
    }

    fn resolve_runs(&mut self) {
        let len = self.types.len();
        self.runs.clear();
        let mut start = 0;
        while start < len {
            if !is_removed_by_x9(self.types[start]) {
                break;
            }
            start += 1;
        }
        if start == len {
            return;
        }
        let mut level = self.levels[start];
        let mut offset = 0;
        for i in start + 1..len {
            if is_removed_by_x9(self.types[i]) {
                continue;
            }
            if self.levels[i] != level {
                self.runs.push(Run::new(level, offset, i));
                offset = i;
                level = self.levels[i];
            }
        }
        if offset < len {
            self.runs.push(Run::new(level, offset, len));
        }
        for run in &mut self.runs {
            while run.start < run.end {
                if is_removed_by_x9(self.types[run.start]) {
                    run.start += 1;
                } else {
                    break;
                }
            }
            while run.end > run.start {
                if is_removed_by_x9(self.types[run.end - 1]) {
                    run.end -= 1;
                } else {
                    break;
                }
            }
            if run.start == run.end {
                continue;
            }
            if self.types[run.start] == BidiClass::PopDirectionalIsolate {
                run.starts_with_pdi = true;
            }
            let mut prev_level = self.base_level;
            for i in (0..run.start).rev() {
                if !is_removed_by_x9(self.types[i]) {
                    prev_level = self.levels[i];
                    break;
                }
            }
            run.sos = type_from_level(prev_level.max(run.level));
            if is_isolate_initiator(self.initial_types[run.end - 1]) {
                run.ends_with_isolate = true;
                run.eos = type_from_level(self.base_level.max(run.level));
            } else {
                let mut next_level = self.base_level;
                for i in run.end..len {
                    if !is_removed_by_x9(self.types[i]) {
                        next_level = self.levels[i];
                        break;
                    }
                }
                run.eos = type_from_level(next_level.max(run.level));
            }
        }
        for i in 0..self.runs.len() {
            if self.runs[i].ends_with_isolate {
                let level = self.runs[i].level;
                for j in i + 1..self.runs.len() {
                    if self.runs[j].starts_with_pdi && self.runs[j].level == level {
                        self.runs[i].next = Some(j);
                        self.runs[j].in_sequence = true;
                        break;
                    }
                }
            }
        }
    }

    #[expect(clippy::needless_range_loop, reason = "Deferred")]
    fn resolve_sequence(&mut self, level: u8, sos: BidiClass, eos: BidiClass, len: usize) {
        if len == 0 {
            return;
        }
        const W1_MASK: u32 = mask(BidiClass::LeftToRightIsolate)
            | mask(BidiClass::RightToLeftIsolate)
            | mask(BidiClass::FirstStrongIsolate)
            | mask(BidiClass::PopDirectionalIsolate);
        const W2_MASK: u32 = mask(BidiClass::LeftToRight)
            | mask(BidiClass::RightToLeft)
            | mask(BidiClass::ArabicLetter);
        const W4_MASK: u32 = mask(BidiClass::EuropeanSeparator) | mask(BidiClass::CommonSeparator);
        let mut prev = sos;
        let mut prev_strong = prev;
        let types = &mut self.types[self.initial_types.len()..];
        for i in 0..len {
            let mut t = types[i];
            let tmask = mask(t);
            if t == BidiClass::NonspacingMark {
                // W1
                types[i] = prev;
            } else {
                if tmask & W1_MASK != 0 {
                    prev = BidiClass::OtherNeutral;
                    continue;
                }
                if t == BidiClass::EuropeanNumber {
                    // W2
                    if prev_strong == BidiClass::ArabicLetter {
                        t = BidiClass::ArabicNumber;
                        types[i] = t;
                    }
                } else if tmask & W2_MASK != 0 {
                    prev_strong = t;
                    // W3
                    if t == BidiClass::ArabicLetter {
                        t = BidiClass::RightToLeft;
                        types[i] = t;
                    }
                } else if tmask & W4_MASK != 0 && i < (len - 1) {
                    // W4
                    let mut next = types[i + 1];
                    if next == BidiClass::EuropeanNumber && prev_strong == BidiClass::ArabicLetter {
                        next = BidiClass::ArabicNumber;
                    }
                    if prev == BidiClass::EuropeanNumber && next == BidiClass::EuropeanNumber {
                        t = BidiClass::EuropeanNumber;
                        types[i] = t;
                    } else if t == BidiClass::CommonSeparator
                        && prev == BidiClass::ArabicNumber
                        && next == BidiClass::ArabicNumber
                    {
                        t = BidiClass::ArabicNumber;
                        types[i] = t;
                    }
                }
                prev = t;
            }
        }
        // W5
        let mut i = 0;
        while i < len {
            if types[i] == BidiClass::EuropeanTerminator {
                let limit = find_limit(types, i, BidiClass::EuropeanTerminator);
                let mut t = if i == 0 { sos } else { types[i - 1] };
                if t != BidiClass::EuropeanNumber {
                    t = if limit == len { eos } else { types[limit] };
                }
                if t == BidiClass::EuropeanNumber {
                    for j in i..limit {
                        types[j] = BidiClass::EuropeanNumber;
                    }
                }
                i = limit;
            }
            i += 1;
        }
        // W6, W7
        const W6_MASK: u32 = mask(BidiClass::EuropeanSeparator)
            | mask(BidiClass::EuropeanTerminator)
            | mask(BidiClass::CommonSeparator);
        prev_strong = sos;
        for i in 0..len {
            let t = types[i];
            if mask(t) & W6_MASK != 0 {
                // W6
                types[i] = BidiClass::OtherNeutral;
            } else if t == BidiClass::EuropeanNumber {
                // W7
                if prev_strong == BidiClass::LeftToRight {
                    types[i] = BidiClass::LeftToRight;
                }
            } else if t == BidiClass::LeftToRight || t == BidiClass::RightToLeft {
                prev_strong = t;
            }
        }
        // N0
        if !self.brackets.is_empty() {
            let base_brackets = self.bracket_pairs.len();
            let mut bracket_stack = BracketStack::new();
            for i in 0..len {
                if types[i] != BidiClass::OtherNeutral {
                    continue;
                }
                let index = self.indices[i];
                if let Ok(index) = self.brackets.binary_search_by(|x| x.0.cmp(&index)) {
                    let (_, ch, bracket) = self.brackets[index];
                    match bracket.paired_bracket_type {
                        BidiPairedBracketType::Open => {
                            if bracket_stack.depth == MAX_BRACKET_STACK {
                                break;
                            }
                            bracket_stack.push(i, bracket.mirroring_glyph.unwrap());
                        }
                        BidiPairedBracketType::Close => {
                            if let Some(open) = bracket_stack.find_and_pop(ch) {
                                self.bracket_pairs.push((open, i));
                            }
                        }
                        _ => {}
                    }
                }
            }
            if self.bracket_pairs.len() > base_brackets {
                let embed_dir = if level & 1 != 0 {
                    BidiClass::RightToLeft
                } else {
                    BidiClass::LeftToRight
                };
                let bracket_pairs = &mut self.bracket_pairs[base_brackets..];
                bracket_pairs.sort_unstable_by_key(|a| a.0);
                for pair in bracket_pairs {
                    let mut pair_dir = BidiClass::OtherNeutral;
                    for i in pair.0 + 1..pair.1 {
                        let dir = match types[i] {
                            BidiClass::EuropeanNumber
                            | BidiClass::ArabicNumber
                            | BidiClass::ArabicLetter
                            | BidiClass::RightToLeft => BidiClass::RightToLeft,
                            BidiClass::LeftToRight => BidiClass::LeftToRight,
                            _ => BidiClass::OtherNeutral,
                        };
                        if dir == BidiClass::OtherNeutral {
                            continue;
                        }
                        pair_dir = dir;
                        if dir == embed_dir {
                            break;
                        }
                    }
                    if pair_dir == BidiClass::OtherNeutral {
                        pair.0 = self.indices[pair.0];
                        pair.1 = self.indices[pair.1];
                        continue;
                    }
                    if pair_dir != embed_dir {
                        pair_dir = sos;
                        for i in (0..pair.0).rev() {
                            let dir = match types[i] {
                                BidiClass::EuropeanNumber
                                | BidiClass::ArabicNumber
                                | BidiClass::ArabicLetter
                                | BidiClass::RightToLeft => BidiClass::RightToLeft,
                                BidiClass::LeftToRight => BidiClass::LeftToRight,
                                _ => BidiClass::OtherNeutral,
                            };
                            if dir != BidiClass::OtherNeutral {
                                pair_dir = dir;
                                break;
                            }
                        }
                        if pair_dir == embed_dir || pair_dir == BidiClass::OtherNeutral {
                            pair_dir = embed_dir;
                        }
                    }
                    types[pair.0] = pair_dir;
                    types[pair.1] = pair_dir;
                    for i in pair.0 + 1..pair.1 {
                        let index = self.indices[i];
                        if self.initial_types[index] == BidiClass::NonspacingMark {
                            types[i] = pair_dir;
                        } else {
                            break;
                        }
                    }
                    for i in pair.1 + 1..len {
                        let index = self.indices[i];
                        if self.initial_types[index] == BidiClass::NonspacingMark {
                            types[i] = pair_dir;
                        } else {
                            break;
                        }
                    }
                    pair.0 = self.indices[pair.0];
                    pair.1 = self.indices[pair.1];
                }
            }
        }
        // N1, N2
        const N_MASK: u32 = mask(BidiClass::ParagraphSeparator)
            | mask(BidiClass::SegmentSeparator)
            | mask(BidiClass::WhiteSpace)
            | mask(BidiClass::OtherNeutral)
            | mask(BidiClass::RightToLeftIsolate)
            | mask(BidiClass::LeftToRightIsolate)
            | mask(BidiClass::FirstStrongIsolate)
            | mask(BidiClass::PopDirectionalIsolate);
        let mut i = 0;
        while i < len {
            let t = types[i];
            if mask(t) & N_MASK != 0 {
                let offset = i;
                let limit = find_limit_by_mask(types, offset, N_MASK);
                let mut leading;
                let mut trailing;
                if offset == 0 {
                    leading = sos;
                } else {
                    leading = types[offset - 1];
                    if leading == BidiClass::ArabicNumber || leading == BidiClass::EuropeanNumber {
                        leading = BidiClass::RightToLeft;
                    }
                }
                if limit == len {
                    trailing = eos;
                } else {
                    trailing = types[limit];
                    if trailing == BidiClass::ArabicNumber || trailing == BidiClass::EuropeanNumber
                    {
                        trailing = BidiClass::RightToLeft;
                    }
                }
                let resolved = if leading == trailing {
                    // N1
                    leading
                } else {
                    // N2
                    if level & 1 != 0 {
                        BidiClass::RightToLeft
                    } else {
                        BidiClass::LeftToRight
                    }
                };
                for j in offset..limit {
                    types[j] = resolved;
                }
                i = limit - 1;
            }
            i += 1;
        }
        // Implicit levels
        if level & 1 == 0 {
            // I1
            for i in 0..len {
                let index = self.indices[i];
                let t = types[i];
                if t == BidiClass::RightToLeft {
                    self.levels[index] = BidiLevel::new(level + 1);
                } else if t != BidiClass::LeftToRight {
                    self.levels[index] = BidiLevel::new(level + 2);
                } else {
                    self.levels[index] = BidiLevel::new(level);
                }
            }
        } else {
            // I2
            for i in 0..len {
                let index = self.indices[i];
                let t = types[i];
                if t != BidiClass::RightToLeft {
                    self.levels[index] = BidiLevel::new(level + 1);
                } else {
                    self.levels[index] = BidiLevel::new(level);
                }
            }
        }
    }
}

/// Returns a default bidi type for a level.
pub(crate) fn type_from_level(level: BidiLevel) -> BidiClass {
    if level.is_ltr() {
        BidiClass::LeftToRight
    } else {
        BidiClass::RightToLeft
    }
}

/// Computes an ordering for a sequence of bidi runs based on levels.
pub(crate) fn _reorder<F>(order: &mut [usize], levels: F)
where
    F: Fn(usize) -> u8,
{
    let mut max_level = 0;
    let mut lowest_odd_level = 255;
    for (i, o) in order.iter_mut().enumerate() {
        *o = i;
        let level = levels(i);
        if level > max_level {
            max_level = level;
        }
        if level & 1 != 0 && level < lowest_odd_level {
            lowest_odd_level = level;
        }
    }
    let len = order.len();
    for level in (lowest_odd_level..=max_level).rev() {
        let mut i = 0;
        while i < len {
            if levels(i) >= level {
                let mut end = i + 1;
                while end < len && levels(end) >= level {
                    end += 1;
                }
                let mut j = i;
                let mut k = end - 1;
                while j < k {
                    order.swap(j, k);
                    j += 1;
                    k -= 1;
                }
                i = end;
            }
            i += 1;
        }
    }
}

/// Returns whether the character needs bidirectional resolution.
#[inline(always)]
pub fn needs_bidi_resolution(bidi_class: BidiClass) -> bool {
    mask(bidi_class) & BIDI_MASK != 0
}

const OVERRIDE_MASK: u32 = mask(BidiClass::RightToLeftEmbedding)
    | mask(BidiClass::LeftToRightEmbedding)
    | mask(BidiClass::RightToLeftOverride)
    | mask(BidiClass::LeftToRightOverride);
const ISOLATE_MASK: u32 = mask(BidiClass::RightToLeftIsolate)
    | mask(BidiClass::LeftToRightIsolate)
    | mask(BidiClass::FirstStrongIsolate);
const EXPLICIT_MASK: u32 = OVERRIDE_MASK | ISOLATE_MASK;
const RTL_MASK: u32 = mask(BidiClass::RightToLeftEmbedding)
    | mask(BidiClass::RightToLeftOverride)
    | mask(BidiClass::RightToLeftIsolate);
const REMOVED_BY_X9_MASK: u32 =
    OVERRIDE_MASK | mask(BidiClass::PopDirectionalFormat) | mask(BidiClass::BoundaryNeutral);
const BIDI_MASK: u32 = EXPLICIT_MASK
    | mask(BidiClass::RightToLeft)
    | mask(BidiClass::ArabicLetter)
    | mask(BidiClass::ArabicNumber);
const STRONG_MASK: u32 =
    mask(BidiClass::LeftToRight) | mask(BidiClass::RightToLeft) | mask(BidiClass::ArabicLetter);
/// Classes that resolve like their surroundings when enclosed by strong characters of the same
/// class: separators and terminators become ON (W6) or EN, and neutrals resolve to the class of
/// the enclosing strong characters (N1). Non-spacing marks take the class of the preceding
/// character (W1) and boundary neutrals take the level of the preceding character (X9).
/// Paired brackets are excluded by the caller as they participate in N0 individually.
const MERGE_NEUTRAL_MASK: u32 = mask(BidiClass::EuropeanSeparator)
    | mask(BidiClass::EuropeanTerminator)
    | mask(BidiClass::CommonSeparator)
    | mask(BidiClass::WhiteSpace)
    | mask(BidiClass::OtherNeutral)
    | mask(BidiClass::NonspacingMark)
    | mask(BidiClass::BoundaryNeutral);

/// The classes that may be merged into a unit started by a strong character of class `class`.
///
/// European numbers preceded by L (with no intervening strong type) resolve to L (W7), but after
/// R they stay EN and after AL they become AN (W2), so they only merge into L units.
fn merge_mask(class: BidiClass) -> u32 {
    if class == BidiClass::LeftToRight {
        mask(class) | MERGE_NEUTRAL_MASK | mask(BidiClass::EuropeanNumber)
    } else {
        mask(class) | MERGE_NEUTRAL_MASK
    }
}

const _RESET_MASK: u32 =
    ISOLATE_MASK | mask(BidiClass::PopDirectionalIsolate) | mask(BidiClass::WhiteSpace);

fn is_isolate_initiator(ty: BidiClass) -> bool {
    mask(ty) & ISOLATE_MASK != 0
}

pub(crate) fn is_removed_by_x9(ty: BidiClass) -> bool {
    mask(ty) & REMOVED_BY_X9_MASK != 0
}

pub(crate) fn _is_reset(ty: BidiClass) -> bool {
    mask(ty) & _RESET_MASK != 0
}

fn find_limit(types: &[BidiClass], offset: usize, ty: BidiClass) -> usize {
    let mut len = offset;
    for &t in &types[offset..] {
        if t != ty {
            break;
        }
        len += 1;
    }
    len
}

fn find_limit_by_mask(types: &[BidiClass], offset: usize, mask: u32) -> usize {
    let mut len = offset;
    for &t in &types[offset..] {
        if self::mask(t) & mask == 0 {
            break;
        }
        len += 1;
    }
    len
}

#[derive(Clone)]
struct Run {
    level: BidiLevel,
    ends_with_isolate: bool,
    starts_with_pdi: bool,
    sos: BidiClass,
    eos: BidiClass,
    start: usize,
    end: usize,
    in_sequence: bool,
    next: Option<usize>,
}

impl Run {
    fn new(level: BidiLevel, start: usize, end: usize) -> Self {
        Self {
            level,
            ends_with_isolate: false,
            starts_with_pdi: false,
            sos: BidiClass::OtherNeutral,
            eos: BidiClass::OtherNeutral,
            start,
            end,
            in_sequence: false,
            next: None,
        }
    }
}

const MAX_STACK: usize = BidiLevel::MAX.to_u8() as usize;

struct Stack {
    embedding_level: [BidiLevel; MAX_STACK + 1],
    override_status: [BidiClass; MAX_STACK + 1],
    isolate_status: [bool; MAX_STACK + 1],
    depth: usize,
}

impl Stack {
    fn new() -> Self {
        Self {
            depth: 0,
            embedding_level: [BidiLevel::new(0); MAX_STACK + 1],
            override_status: [BidiClass::OtherNeutral; MAX_STACK + 1],
            isolate_status: [false; MAX_STACK + 1],
        }
    }

    fn push(&mut self, level: BidiLevel, override_status: BidiClass, isolate_status: bool) {
        let d = self.depth;
        self.embedding_level[d] = level;
        self.override_status[d] = override_status;
        self.isolate_status[d] = isolate_status;
        self.depth += 1;
    }

    fn pop(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
        }
    }

    fn embedding_level(&self) -> BidiLevel {
        self.embedding_level[self.depth - 1]
    }

    fn override_status(&self) -> BidiClass {
        self.override_status[self.depth - 1]
    }

    fn isolate_status(&self) -> bool {
        self.isolate_status[self.depth - 1]
    }
}

const MAX_BRACKET_STACK: usize = 63;

struct BracketStack {
    openers: [(usize, char); MAX_BRACKET_STACK],
    depth: usize,
}

impl BracketStack {
    fn new() -> Self {
        Self {
            openers: [(0, '\0'); MAX_BRACKET_STACK],
            depth: 0,
        }
    }

    fn push(&mut self, offset: usize, closer: char) {
        self.openers[self.depth] = (offset, closer);
        self.depth += 1;
    }

    fn find_and_pop(&mut self, closer: char) -> Option<usize> {
        if self.depth == 0 {
            return None;
        }
        for i in (0..self.depth).rev() {
            let c = self.openers[i].1;
            if c == closer
                || (c == '\u{232A}' && closer == '\u{3009}')
                || (c == '\u{3009}' && closer == '\u{232A}')
            {
                self.depth = i;
                return Some(self.openers[i].0);
            }
        }
        None
    }
}

/// Turns a bidi class into a single bit, for cheap set membership testing.
const fn mask(t: BidiClass) -> u32 {
    let bit = match t {
        BidiClass::LeftToRight => 0,
        BidiClass::RightToLeft => 1,
        BidiClass::EuropeanNumber => 2,
        BidiClass::EuropeanSeparator => 3,
        BidiClass::EuropeanTerminator => 4,
        BidiClass::ArabicNumber => 5,
        BidiClass::CommonSeparator => 6,
        BidiClass::ParagraphSeparator => 7,
        BidiClass::SegmentSeparator => 8,
        BidiClass::WhiteSpace => 9,
        BidiClass::OtherNeutral => 10,
        BidiClass::LeftToRightEmbedding => 11,
        BidiClass::LeftToRightOverride => 12,
        BidiClass::ArabicLetter => 13,
        BidiClass::RightToLeftEmbedding => 14,
        BidiClass::RightToLeftOverride => 15,
        BidiClass::PopDirectionalFormat => 16,
        BidiClass::NonspacingMark => 17,
        BidiClass::BoundaryNeutral => 18,
        BidiClass::FirstStrongIsolate => 19,
        BidiClass::LeftToRightIsolate => 20,
        BidiClass::RightToLeftIsolate => 21,
        BidiClass::PopDirectionalIsolate => 22,
        _ => {
            debug_assert!(false, "unhandled Bidi_Class");
            return 0;
        }
    };
    1 << bit
}

#[cfg(test)]
mod test {
    use alloc::vec::Vec;
    use icu_properties::CodePointMapData;
    use icu_properties::props::{BidiClass, BidiMirroringGlyph, BidiPairedBracketType};
    use parlance::BaseDirection;
    use parley_data::Properties;

    use super::BidiResolver;

    fn index(r: u64) -> usize {
        usize::try_from(r).unwrap()
    }

    fn pick(r: u64) -> char {
        POOL[index(r % POOL.len() as u64)]
    }

    /// Characters covering every `Bidi_Class`, paired brackets, and explicit formatting characters.
    const POOL: &[char] = &[
        'a', 'b', 'Z', '\u{05D0}', '\u{05D1}', '\u{0627}', '\u{0628}', '0', '9', '\u{00B2}',
        '\u{0660}', '\u{0663}', '\u{0600}', '+', '-', '#', '$', '%', '\u{00B0}', ',', '.', ':',
        '/', '\u{00A0}', '\u{0300}', '\u{064B}', '\u{00AD}', '\u{200D}', '\u{0000}', '\n',
        '\u{2029}', '\u{001C}', '\t', '\u{001F}', ' ', '\u{2003}', '!', '"', '&', '*', '<', '>',
        '(', ')', '[', ']', '{', '}', '\u{2329}', '\u{232A}', '\u{3008}', '\u{3009}', '\u{202A}',
        '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}',
        '\u{2069}',
    ];

    fn resolve(text: &[char], direction: BaseDirection, merge: bool) -> BidiResolver {
        let brackets = CodePointMapData::<BidiMirroringGlyph>::new();
        let mut resolver = BidiResolver::new();
        resolver.resolve_impl(
            text.iter()
                .map(|&ch| (ch, (Properties::get(ch).bidi_class(), brackets.get(ch)))),
            direction,
            merge,
        );
        resolver
    }

    /// Merging runs of characters into units must not change any resolved level.
    #[test]
    fn merged_units_match_per_character_resolution() {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut text = Vec::new();
        let mut merged_texts = 0;
        for iteration in 0..40_000 {
            text.clear();
            let len = if iteration % 100 == 0 {
                index(next() % 600)
            } else {
                index(next() % 40)
            };
            // Bias each text towards a few classes so that long mergeable runs occur.
            let favored = [
                pick(next()),
                pick(next()),
                *[' ', 'a', '\u{05D0}', '\u{0627}'][index(next() % 4)..]
                    .first()
                    .unwrap(),
            ];
            for _ in 0..len {
                let r = next();
                text.push(if r % 3 == 0 {
                    pick(r >> 8)
                } else {
                    favored[index((r >> 8) % 3)]
                });
            }
            for direction in [BaseDirection::Auto, BaseDirection::Ltr, BaseDirection::Rtl] {
                let merged = resolve(&text, direction, true);
                let reference = resolve(&text, direction, false);
                if merged.initial_types.len() < text.len() {
                    merged_texts += 1;
                }
                assert_eq!(merged.levels().len(), text.len());
                assert_eq!(
                    (merged.base_level(), merged.levels()),
                    (reference.base_level(), reference.levels()),
                    "{text:?} {direction:?}"
                );
            }
        }
        assert!(
            merged_texts > 40_000,
            "too few texts exercised merging: {merged_texts}"
        );
    }

    /// Bracket pair lookups are skipped for characters whose `Bidi_Class` is not ON, which
    /// relies on every paired bracket being ON.
    #[test]
    fn paired_brackets_are_other_neutral() {
        let brackets = CodePointMapData::<BidiMirroringGlyph>::new();
        for ch in (0..=char::MAX as u32).filter_map(char::from_u32) {
            if brackets.get(ch).paired_bracket_type != BidiPairedBracketType::None {
                assert_eq!(
                    Properties::get(ch).bidi_class(),
                    BidiClass::OtherNeutral,
                    "{ch:?} is a paired bracket but not ON"
                );
            }
        }
    }
}
