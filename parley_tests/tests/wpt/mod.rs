// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Lays out text blocks recorded from Web Platform Tests and checks that every
//! character lands where Chrome put it.
//!
//! The fixtures under `fixtures/` (and the fonts they use under `fonts/`) are written
//! by `parley_wpt_recorder` from a WPT checkout and a Chrome; see `README.md` in this
//! directory. This test needs neither: it is a plain fixture runner.
//!
//! Every fixture has one of four outcomes:
//!
//! - *skipped*: it uses a CSS feature Parley does not implement (`check_supported`);
//! - *pass*: every character's rect matches Chrome's within `Tolerance`;
//! - *fail*: some character's rect differs;
//! - *error*: the fixture could not be laid out (a property value the adapter in
//!   `parley_wpt_cases` does not understand, a missing font).
//!
//! `expectations.txt` lists the fixtures known to fail or error. The test fails when a
//! fixture's outcome differs from what is expected in either direction, so a fix that
//! makes fixtures pass has to be recorded too. Run with `PARLEY_TEST=accept` to rewrite
//! `expectations.txt` from the actual outcomes, and set `PARLEY_WPT_FILTER=<substring>`
//! to run only matching fixtures and print their diffs.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use parley::LayoutContext;
use parley_wpt_cases::{
    Case, FontFiles, Tolerance, check_supported, compare_chars, font_context, layout, parley_chars,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Status {
    Pass,
    Fail,
    Error,
}

impl Status {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "PASS" => Some(Self::Pass),
            "FAIL" => Some(Self::Fail),
            "ERROR" => Some(Self::Error),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Error => "ERROR",
        }
    }
}

enum Outcome {
    Skipped(String),
    Pass,
    Fail(String),
    Error(String),
}

impl Outcome {
    fn status(&self) -> Option<Status> {
        match self {
            Self::Skipped(_) => None,
            Self::Pass => Some(Status::Pass),
            Self::Fail(_) => Some(Status::Fail),
            Self::Error(_) => Some(Status::Error),
        }
    }

    fn detail(&self) -> &str {
        match self {
            Self::Pass => "",
            Self::Skipped(detail) | Self::Fail(detail) | Self::Error(detail) => detail,
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wpt")
}

fn collect_fixtures(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_fixtures(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "txt") {
            let relative = path.strip_prefix(root).expect("fixture is under root");
            out.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Non-passing fixtures, from `expectations.txt`: `<STATUS> <fixture>` per line.
fn load_expectations(path: &Path) -> BTreeMap<String, Status> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (status, fixture) = line
                .split_once(' ')
                .unwrap_or_else(|| panic!("{}: bad line {line:?}", path.display()));
            let status = Status::parse(status)
                .unwrap_or_else(|| panic!("{}: bad status {status:?}", path.display()));
            (fixture.to_string(), status)
        })
        .collect()
}

fn write_expectations(path: &Path, outcomes: &BTreeMap<String, Status>) {
    let mut text = String::from(
        "# Fixtures that do not currently match Chrome. Regenerate with\n\
         # `PARLEY_TEST=accept cargo test -p parley_tests wpt`.\n",
    );
    for (fixture, status) in outcomes {
        if *status != Status::Pass {
            writeln!(text, "{} {fixture}", status.name()).unwrap();
        }
    }
    std::fs::write(path, text).expect("writing expectations");
}

fn run_case(case: &Case, fonts: &mut FontFiles, layout_cx: &mut LayoutContext<()>) -> Outcome {
    if let Err(unsupported) = check_supported(case) {
        return Outcome::Skipped(unsupported.to_string());
    }
    let mut font_cx = match font_context(case, fonts) {
        Ok(font_cx) => font_cx,
        Err(error) => return Outcome::Error(error.to_string()),
    };
    let layout = match layout(case, &mut font_cx, layout_cx) {
        Ok(layout) => layout,
        Err(error) => return Outcome::Error(error.to_string()),
    };
    let parley = match parley_chars(case, &layout) {
        Ok(parley) => parley,
        Err(error) => return Outcome::Error(error.to_string()),
    };
    match compare_chars(case, &parley, Tolerance::default()) {
        Ok(()) => Outcome::Pass,
        Err(mismatch) => Outcome::Fail(mismatch.to_string()),
    }
}

#[test]
fn wpt_matches_chrome() {
    let root = root();
    let fixtures_dir = root.join("fixtures");
    let mut fixtures = Vec::new();
    collect_fixtures(&fixtures_dir, &fixtures_dir, &mut fixtures);
    fixtures.sort();
    assert!(
        !fixtures.is_empty(),
        "no fixtures under {}",
        fixtures_dir.display()
    );

    let filter = std::env::var("PARLEY_WPT_FILTER").ok();
    let accept = std::env::var("PARLEY_TEST").is_ok_and(|mode| mode == "accept");
    let expectations_path = root.join("expectations.txt");
    let expected = load_expectations(&expectations_path);

    let mut fonts = FontFiles::new(&root.join("fonts"));
    let mut layout_cx = LayoutContext::new();
    let mut statuses: BTreeMap<String, Status> = BTreeMap::new();
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let mut unexpected = String::new();
    let mut unexpected_count = 0;

    for fixture in &fixtures {
        if filter.as_ref().is_some_and(|f| !fixture.contains(f)) {
            continue;
        }
        let text = std::fs::read_to_string(fixtures_dir.join(fixture)).expect("reading fixture");
        let case = Case::parse(&text).unwrap_or_else(|error| panic!("{fixture}: {error}"));
        let outcome = run_case(&case, &mut fonts, &mut layout_cx);
        let Some(status) = outcome.status() else {
            *skipped.entry(outcome.detail().to_string()).or_default() += 1;
            continue;
        };
        statuses.insert(fixture.clone(), status);
        let expected_status = expected.get(fixture).copied().unwrap_or(Status::Pass);
        if status != expected_status || filter.is_some() {
            if status != expected_status {
                unexpected_count += 1;
            }
            writeln!(
                unexpected,
                "{fixture}: {} (expected {})\n  {}\n{}",
                status.name(),
                expected_status.name(),
                case.path,
                indent(outcome.detail())
            )
            .unwrap();
        }
    }

    let count = |status| statuses.values().filter(|s| **s == status).count();
    eprintln!(
        "wpt: {} fixtures: {} pass, {} fail, {} error, {} skipped",
        fixtures.len(),
        count(Status::Pass),
        count(Status::Fail),
        count(Status::Error),
        skipped.values().sum::<usize>()
    );
    let mut reasons: Vec<_> = skipped.into_iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (reason, count) in reasons {
        eprintln!("  skipped {count:4}  {reason}");
    }
    if !unexpected.is_empty() {
        eprintln!("{unexpected}");
    }

    if accept && filter.is_none() {
        // Keep expectations for fixtures that were not run (none, when unfiltered) and
        // drop ones whose fixture no longer exists.
        write_expectations(&expectations_path, &statuses);
        return;
    }
    assert_eq!(
        unexpected_count, 0,
        "{unexpected_count} fixture(s) did not match expectations.txt; \
         see above, and run with PARLEY_TEST=accept to update it"
    );
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}
