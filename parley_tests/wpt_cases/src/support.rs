// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Which recorded cases Parley can be asked to reproduce at all.
//!
//! A case using a CSS feature Parley has no notion of (vertical writing modes,
//! `text-transform`, `hyphens: auto`, ...) is skipped by the test rather than listed as
//! a failure: a failure means "Parley disagrees with Chrome about something it
//! implements". Extend this as features land.

use crate::Case;

/// Why a case cannot be laid out with Parley.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported(pub String);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Container properties that must have (one of) the given values.
const CONTAINER_REQUIREMENTS: &[(&str, &[&str])] = &[
    ("writing-mode", &["horizontal-tb", ""]),
    ("text-orientation", &["mixed", ""]),
    ("text-align-last", &["auto", ""]),
    ("text-wrap-style", &["auto", ""]),
    ("text-justify", &["auto", ""]),
    ("text-overflow", &["clip", ""]),
    ("unicode-bidi", &["normal", "embed", "isolate", ""]),
    ("hanging-punctuation", &["none", ""]),
    ("text-box-trim", &["none", ""]),
    ("text-fit", &["none", ""]),
    ("text-grow", &["none", ""]),
    ("text-shrink", &["none", ""]),
];

/// Run properties that must have (one of) the given values.
const RUN_REQUIREMENTS: &[(&str, &[&str])] = &[
    ("text-transform", &["none", ""]),
    ("text-combine-upright", &["none", ""]),
    ("vertical-align", &["baseline", ""]),
    ("unicode-bidi", &["normal", "embed", "isolate", ""]),
    ("font-variant-caps", &["normal", ""]),
    ("font-variant-numeric", &["normal", ""]),
    ("font-variant-east-asian", &["normal", ""]),
    (
        "white-space-collapse",
        &["collapse", "preserve", "preserve-breaks", ""],
    ),
    ("text-wrap-mode", &["wrap", "nowrap", ""]),
    ("word-break", &["normal", "break-all", "keep-all", ""]),
    ("overflow-wrap", &["normal", "anywhere", "break-word", ""]),
    ("line-break", &["auto", ""]),
    ("hyphens", &["manual", "none", ""]),
    ("initial-letter", &["normal", ""]),
    ("text-emphasis-style", &["none", ""]),
    ("text-justify", &["auto", ""]),
];

/// Length properties whose computed value Chrome reports unresolved when it is a
/// percentage (of the font size, space advance or container width) or a `calc()`.
const RESOLVED_LENGTHS: &[&str] = &["text-indent", "letter-spacing", "word-spacing"];

/// Checks `case` against the feature set Parley implements.
pub fn check_supported(case: &Case) -> Result<(), Unsupported> {
    for (name, allowed) in CONTAINER_REQUIREMENTS {
        let value = case.container_property(name);
        if !allowed.contains(&value) {
            return Err(Unsupported(format!("{name}: {value}")));
        }
    }
    for name in RESOLVED_LENGTHS {
        let value = case.container_property(name);
        if value.contains('%') || value.contains('(') {
            return Err(Unsupported(format!("{name}: {value}")));
        }
    }
    for run in &case.runs {
        for (name, allowed) in RUN_REQUIREMENTS {
            let value = case.run_property(run, name);
            if !allowed.contains(&value) {
                return Err(Unsupported(format!("{name}: {value}")));
            }
        }
        for name in RESOLVED_LENGTHS {
            let value = case.run_property(run, name);
            if value.contains('%') || value.contains('(') {
                return Err(Unsupported(format!("{name}: {value}")));
            }
        }
        // Chrome draws other control characters as hex boxes; Parley drops them.
        if run
            .text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
        {
            return Err(Unsupported("control characters".into()));
        }
        if case.run_property(run, "hyphens") == "manual" && run.text.contains('\u{00AD}') {
            return Err(Unsupported("soft hyphens".into()));
        }
        // Parley has no tab stops: a preserved tab is just its glyph's advance.
        if run.text.contains('\t') && case.run_property(run, "white-space-collapse") == "preserve" {
            return Err(Unsupported("preserved tabs".into()));
        }
    }
    Ok(())
}
