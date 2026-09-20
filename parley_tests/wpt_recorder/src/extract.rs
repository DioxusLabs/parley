// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Turning `extract.js`'s description of a page into fixture [`Case`]s.
//!
//! Not every block the script finds becomes a case. A block is skipped when the script
//! flagged a problem (non-text inline content), when a run's first `font-family` is
//! not a font the page loaded via `@font-face` from a file we can ship (system fonts
//! differ between machines, so Parley could never be given the same font), or when
//! that font lacks a glyph the text needs (Chrome would have fallen back to a system
//! font). Skips are reported so the coverage of a suite is visible.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parley_wpt_cases::{Case, CharRect, FontFace, Run, StyleMap};
use read_fonts::types::{GlyphId, Tag};
use read_fonts::{FontRef, TableProvider};
use serde::Deserialize;

/// Fonts larger than this are not copied into the corpus.
const MAX_FONT_BYTES: u64 = 512 * 1024;

/// Blocks with more code points than this are skipped: they are prose, not tests.
const MAX_TEXT_CHARS: usize = 2000;

#[derive(Debug, Deserialize)]
pub(crate) struct Page {
    pub(crate) fonts: Vec<PageFont>,
    pub(crate) blocks: Vec<Block>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PageFont {
    pub(crate) family: String,
    pub(crate) urls: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Block {
    pub(crate) index: usize,
    pub(crate) path: String,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) lang: String,
    pub(crate) style: StyleMap,
    pub(crate) runs: Vec<BlockRun>,
    pub(crate) problems: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BlockRun {
    pub(crate) text: String,
    pub(crate) lang: String,
    pub(crate) style: StyleMap,
    /// Per code point: zero or more `[x, y, width, height]`.
    pub(crate) chars: Vec<Vec<[f64; 4]>>,
}

/// Why a block was not exported.
#[derive(Debug)]
pub(crate) struct Skipped {
    pub(crate) block: usize,
    pub(crate) path: String,
    pub(crate) reason: String,
}

/// Copies the fonts cases reference into the corpus' `fonts/` directory, naming them
/// by their file name (disambiguated by content if two differ).
pub(crate) struct FontStore {
    wpt_dir: PathBuf,
    fonts_dir: PathBuf,
    /// WPT path → corpus file name, or `None` if the font is unusable.
    known: BTreeMap<String, Option<String>>,
    /// Corpus file name → bytes.
    bytes: BTreeMap<String, Vec<u8>>,
}

impl FontStore {
    pub(crate) fn new(wpt_dir: &Path, fonts_dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(fonts_dir)?;
        let mut store = Self {
            wpt_dir: wpt_dir.to_path_buf(),
            fonts_dir: fonts_dir.to_path_buf(),
            known: BTreeMap::new(),
            bytes: BTreeMap::new(),
        };
        for entry in std::fs::read_dir(fonts_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let name = entry.file_name().to_string_lossy().into_owned();
                store.bytes.insert(name, std::fs::read(entry.path())?);
            }
        }
        Ok(store)
    }

    /// The corpus file for the font at WPT path `url_path`, copying it if needed.
    fn file_for(&mut self, url_path: &str) -> Result<String, String> {
        if let Some(known) = self.known.get(url_path) {
            return known
                .clone()
                .ok_or_else(|| format!("{url_path} is not a usable font"));
        }
        let result = self.import(url_path);
        self.known
            .insert(url_path.to_string(), result.as_ref().ok().cloned());
        result
    }

    fn import(&mut self, url_path: &str) -> Result<String, String> {
        let relative = url_path.trim_start_matches('/');
        let source = self.wpt_dir.join(relative);
        let extension = source
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "ttf" | "otf") {
            return Err(format!(
                "{url_path}: only TrueType/OpenType fonts are supported"
            ));
        }
        let metadata =
            std::fs::metadata(&source).map_err(|error| format!("{}: {error}", source.display()))?;
        if metadata.len() > MAX_FONT_BYTES {
            return Err(format!("{url_path}: {} bytes is too large", metadata.len()));
        }
        let bytes = std::fs::read(&source).map_err(|error| format!("{url_path}: {error}"))?;
        FontRef::new(&bytes).map_err(|error| format!("{url_path}: not a font: {error}"))?;

        let base = source
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("{url_path}: bad file name"))?
            .to_string();
        let mut name = base.clone();
        let mut attempt = 1;
        loop {
            match self.bytes.get(&name) {
                Some(existing) if *existing == bytes => break,
                Some(_) => {
                    attempt += 1;
                    let (stem, ext) = base.rsplit_once('.').unwrap_or((&base, ""));
                    name = format!("{stem}-{attempt}.{ext}");
                }
                None => {
                    std::fs::write(self.fonts_dir.join(&name), &bytes)
                        .map_err(|error| format!("writing {name}: {error}"))?;
                    self.bytes.insert(name.clone(), bytes);
                    break;
                }
            }
        }
        Ok(name)
    }

    fn bytes(&self, file: &str) -> &[u8] {
        &self.bytes[file]
    }
}

/// The first family of a computed `font-family` list, unquoted.
fn first_family(value: &str) -> &str {
    let first = value.split(',').next().unwrap_or("").trim();
    first
        .strip_prefix('"')
        .and_then(|f| f.strip_suffix('"'))
        .or_else(|| first.strip_prefix('\'').and_then(|f| f.strip_suffix('\'')))
        .unwrap_or(first)
}

/// Code points Chrome renders as nothing whatever the font (white space that
/// collapses or breaks, controls, formatting characters), so their absence from the
/// font's `cmap` does not trigger fallback. Spaces are not among them: a font without
/// a space glyph gets its spaces from a fallback font.
fn is_ignorable(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r')
        || c.is_control()
        || matches!(c, '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}' | '\u{FEFF}' | '\u{FE00}'..='\u{FE0F}')
}

/// The first character of `text` the font has no glyph for. Shapers synthesise
/// no-break spaces from the space glyph, so U+00A0 only needs U+0020.
fn missing_glyph(font: &[u8], text: &str) -> Option<char> {
    let font = FontRef::new(font).ok()?;
    let cmap = font.cmap().ok()?;
    let has_glyph = |c: char| {
        cmap.map_codepoint(c)
            .is_some_and(|glyph| glyph != GlyphId::NOTDEF)
    };
    text.chars()
        .find(|&c| !is_ignorable(c) && !has_glyph(c) && !(c == '\u{00A0}' && has_glyph(' ')))
}

/// Fullwidth CJK punctuation (and the quotation marks CJK fonts draw fullwidth) whose
/// spacing `text-spacing-trim: normal` — Chrome's default, applying to all text — trims
/// at line starts and between adjacent punctuation.
fn is_trimmable_punctuation(c: char) -> bool {
    matches!(
        c,
        '\u{2018}'..='\u{201D}'
            | '\u{3001}'..='\u{3003}'
            | '\u{3008}'..='\u{3011}'
            | '\u{3014}'..='\u{301F}'
            | '\u{FF01}'..='\u{FF0F}'
            | '\u{FF1A}'..='\u{FF1F}'
            | '\u{FF3B}'..='\u{FF3D}'
            | '\u{FF5B}'..='\u{FF60}'
    )
}

/// Whether the font has the GPOS features Chrome implements `text-spacing-trim` with;
/// without them Chrome leaves punctuation spacing alone, as Parley always does.
fn has_spacing_features(font: &[u8]) -> bool {
    const FEATURES: [Tag; 4] = [
        Tag::new(b"chws"),
        Tag::new(b"halt"),
        Tag::new(b"vchw"),
        Tag::new(b"vhal"),
    ];
    FontRef::new(font)
        .ok()
        .and_then(|font| font.gpos().ok())
        .and_then(|gpos| gpos.feature_list().ok())
        .is_some_and(|features| {
            features
                .feature_records()
                .iter()
                .any(|record| FEATURES.contains(&record.feature_tag()))
        })
}

/// Properties Chrome gave no value for (ones it does not implement) are omitted.
fn without_empty(style: &StyleMap) -> StyleMap {
    style
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// Converts the blocks of `page` (at WPT path `source`) into cases.
pub(crate) fn cases(source: &str, page: &Page, fonts: &mut FontStore) -> (Vec<Case>, Vec<Skipped>) {
    let mut cases = Vec::new();
    let mut skipped = Vec::new();
    for block in &page.blocks {
        match convert(source, page, block, fonts) {
            Ok(case) => cases.push(case),
            Err(reason) => skipped.push(Skipped {
                block: block.index,
                path: block.path.clone(),
                reason,
            }),
        }
    }
    (cases, skipped)
}

fn convert(
    source: &str,
    page: &Page,
    block: &Block,
    fonts: &mut FontStore,
) -> Result<Case, String> {
    if let Some(problem) = block.problems.first() {
        return Err(problem.clone());
    }
    let total_chars: usize = block.runs.iter().map(|run| run.text.chars().count()).sum();
    if total_chars > MAX_TEXT_CHARS {
        return Err(format!("{total_chars} characters of text"));
    }

    let container = without_empty(&block.style);
    let mut faces: Vec<FontFace> = Vec::new();
    let mut styles: Vec<StyleMap> = Vec::new();
    let mut runs = Vec::new();
    let mut chars = Vec::new();

    for run in &block.runs {
        if run.text.is_empty() {
            continue;
        }
        let family = first_family(run.style.get("font-family").map_or("", String::as_str));
        if run.text.chars().any(|c| !is_ignorable(c)) {
            let page_font = page
                .fonts
                .iter()
                .find(|font| font.family.eq_ignore_ascii_case(family))
                .ok_or_else(|| format!("font-family {family:?} is not an @font-face"))?;
            let mut file = Err(format!("{family}: no usable @font-face source"));
            for url in &page_font.urls {
                file = fonts.file_for(url);
                if file.is_ok() {
                    break;
                }
            }
            let file = file?;
            if let Some(c) = missing_glyph(fonts.bytes(&file), &run.text) {
                return Err(format!("{family} has no glyph for U+{:04X}", u32::from(c)));
            }
            if run.text.chars().any(is_trimmable_punctuation)
                && has_spacing_features(fonts.bytes(&file))
            {
                return Err(format!("text-spacing-trim: {family} has chws/halt"));
            }
            if !faces
                .iter()
                .any(|face| face.family == family && face.file == file)
            {
                faces.push(FontFace {
                    family: family.to_string(),
                    file,
                });
            }
        }

        let mut run_style = without_empty(&run.style);
        run_style.retain(|name, value| container.get(name) != Some(value));
        let style = match styles.iter().position(|s| *s == run_style) {
            Some(index) => index,
            None => {
                styles.push(run_style);
                styles.len() - 1
            }
        };
        let index = runs.len();
        runs.push(Run {
            text: run.text.clone(),
            lang: run.lang.clone(),
            style,
        });
        for (char_index, rects) in run.chars.iter().enumerate() {
            for rect in rects {
                chars.push(CharRect {
                    run: index,
                    index: char_index,
                    x: rect[0],
                    y: rect[1],
                    width: rect[2],
                    height: rect[3],
                });
            }
        }
    }

    if chars.is_empty() {
        return Err("no character has a rect".into());
    }

    Ok(Case {
        source: source.to_string(),
        block: block.index,
        path: block.path.clone(),
        width: block.width,
        height: block.height,
        lang: block.lang.clone(),
        fonts: faces,
        container,
        styles,
        runs,
        chars,
    })
}
