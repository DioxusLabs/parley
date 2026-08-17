# Running WPT tests against Parley (via Blitz)

[Blitz](https://github.com/DioxusLabs/blitz) is an HTML/CSS rendering engine that uses Parley
for all of its text layout. It ships a [Web Platform Tests](https://github.com/web-platform-tests/wpt)
(WPT) runner which renders each test and its reference to a PNG and compares the two images.
Because the text-related WPT suites (`css/css-text`, `css/css-fonts`, etc.) exercise line
breaking, shaping, bidi, font fallback and font selection, this is a convenient way to test
Parley changes against thousands of real-world text layout tests.

This document explains how to run the Blitz WPT runner against a **local checkout of Parley**.

## Prerequisites

1. **A Rust toolchain** (stable). Blitz builds with recent stable Rust.
2. **A clone of Blitz**:
   ```sh
   git clone https://github.com/DioxusLabs/blitz.git
   ```
3. **A clone of the WPT test suite** (large, a shallow clone is fine):
   ```sh
   git clone --depth 1 https://github.com/web-platform-tests/wpt.git
   ```
4. **Your local Parley checkout** (this repository).

The layout assumed by the rest of this document (and by the commented-out patch section in
Blitz's `Cargo.toml`) is sibling directories:

```
~/code/blitz
~/code/parley
~/code/wpt
```

## Pointing Blitz at your local Parley

Blitz normally depends on a released version of Parley from crates.io. To test local changes,
override it with a `[patch]` section. At the bottom of Blitz's root `Cargo.toml` there is a
commented-out template for this. Enable it like so:

```toml
[patch.crates-io]
parley = { path = "../parley/parley" }
fontique = { path = "../parley/fontique" }
```

(`fontique` only strictly needs patching if you are changing it, but patching both keeps the
two in sync.)

### Version compatibility gotchas

- **The patched version must semver-match the version Blitz requires.** Cargo only applies a
  `[patch]` if the patched crate's version satisfies the dependency requirement. If Blitz
  depends on `parley = "0.11.1"` but your checkout's workspace version is `0.11.0`, the patch
  is silently *not used* (Cargo prints a `patch ... was not used in the crate graph` warning).
  Fix this by temporarily bumping `version` in Parley's root `Cargo.toml` (the
  `[workspace.package]` `version` key) to match — no need to commit this change.

- **Blitz tracks Parley *releases*, not Parley `main`.** If your branch is based on `main` and
  Parley's API has moved on since the last release, Blitz may fail to compile against it with
  small API-mismatch errors (renamed methods, new struct fields, changed signatures). Options:
  - Base your Parley branch on the branch/tag matching the release Blitz uses
    (e.g. `v0.11.x`), or
  - Fix up the (usually small) API mismatches in Blitz locally, or
  - Check whether Blitz has a branch that already tracks a newer Parley.

You can verify the patch took effect with:

```sh
cargo tree -p parley    # should print: parley vX.Y.Z (/path/to/your/parley/parley)
```

## Running the tests

The runner needs the `WPT_DIR` environment variable pointing at your WPT clone. From the Blitz
repo root:

```sh
WPT_DIR=../wpt cargo run --release --package wpt -- css/css-text
```

or equivalently, using the `just` recipe (`just wpt <filter>`):

```sh
export WPT_DIR=../wpt
just wpt css/css-text
```

The positional argument(s) are path filters relative to the WPT root. You can pass:

- a directory: `css/css-text/word-break`
- multiple suites: `css/css-text css/css-fonts`
- a single test file: `css/css-text/word-break/word-break-normal-ja-000.html`

If no filter is given, it defaults to `css/css-flexbox` and `css/css-grid` (layout suites),
so for Parley work you'll always want to pass a text-related filter.

### Suites most relevant to Parley

| Suite | Exercises |
| --- | --- |
| `css/css-text` | Line breaking, `word-break`, `overflow-wrap`, `white-space`, `text-align`, letter/word spacing |
| `css/css-fonts` | Font selection, fallback, `font-variant`, weights/styles |
| `css/css-text-decor` | Underlines, `text-decoration`, `text-emphasis` |
| `css/css-inline` | Inline layout, baselines, `line-height`, `vertical-align` |
| `css/css-writing-modes` | Vertical text, bidi, `direction` |
| `css/css-ruby` | Ruby annotation layout |

Note that failures in these suites are not necessarily Parley bugs — the test may exercise
CSS features that Blitz doesn't implement (yet), or the bug may be in Blitz's inline layout
integration (`blitz-dom`'s "inline root" construction) rather than in Parley itself.

### Useful flags and environment variables

- `-v` / `--verbose`: print each test result as it completes (instead of a progress display).
- `RUST_LOG=info`: enable the runner's logging (it uses `env_logger`).
- The runner is parallel (rayon); set `RAYON_NUM_THREADS=1` for deterministic single-threaded
  runs when debugging.

## Interpreting the output

Each test renders at 800x600 and is compared pixel-for-pixel against its reference(s),
honouring any `<meta name=fuzzy>` tolerances declared by the test. At the end of a run you get
a summary like:

```
 105 tests FOUND
   1 tests SKIPPED (0.95%)
 104 tests RUN (99.05%)
  39 tests PASSED (37.50% of run; 37.14% of found)
  65 tests FAILED (62.50% of run; 61.90% of found)

Of those tests which failed:
  22 do not use unsupported features
   4 use floats (F)
   9 use intrinsic size keywords (I)
  30 use script (X)
```

The runner supports three kinds of test: reftests (`REF`, image comparison against a
reference page), attr tests (`ATT`, `checkLayout()`-style tests whose expectations are encoded
in `data-expected-*` attributes and which are evaluated without running JS), and crashtests
(`CRA`, pass if they render without panicking). `testharness.js` tests (`HAR`) require a JS
engine and are skipped.

The single-letter flags after each result (`F`, `I`, `C`, `D`, `W`, `X`, ...) mark tests that
use features Blitz doesn't fully support (floats, intrinsic sizing keywords, calc, direction,
writing modes, script). Failures marked `X` (script) are often false failures — the runner
does not execute JavaScript — so focus on failures *without* flags first.

### Artifacts in `wpt/output/`

Each run wipes and repopulates `wpt/output/` in the Blitz repo:

- `<test>.html-test.png` — Blitz's rendering of the test page
- `<test>.html-ref.png` (or `-ref-N.png`) — rendering of the reference page(s)
- `<test>.html-diff.png` — pixel diff, written for failing comparisons
- `wpt_expectations.txt` — one line per test: `<path> PASS|FAIL|SKIP|CRASH` (plus a `Y`/`N`
  character per subtest for testharness tests). Handy for diffing two runs.
- `wptreport.json` — standard "WPT report" format, consumable by WPT tooling and dashboards.

## A typical Parley-change workflow

1. Set up the `[patch.crates-io]` override as above.
2. Run the relevant suite **before** your change and save the expectations:
   ```sh
   WPT_DIR=../wpt cargo run --release --package wpt -- css/css-text
   cp wpt/output/wpt_expectations.txt /tmp/before.txt
   ```
3. Make your Parley change (Cargo will pick it up automatically via the path patch).
4. Re-run and diff:
   ```sh
   WPT_DIR=../wpt cargo run --release --package wpt -- css/css-text
   diff /tmp/before.txt wpt/output/wpt_expectations.txt
   ```
5. For any regression, open the `-test.png`, `-ref.png` and `-diff.png` images for that test
   in `wpt/output/` to see what changed visually. The test itself lives in your WPT clone and
   can also be viewed at `https://wpt.live/<test path>` for comparison against real browsers.
