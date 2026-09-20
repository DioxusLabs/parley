# WPT fixtures

Text blocks from the [Web Platform Tests](https://github.com/web-platform-tests/wpt) CSS
text suites, as Chrome lays them out, replayed against Parley by `tests/wpt/mod.rs`
(`cargo test -p parley_tests wpt`). The test runs offline from the checked-in files in
this directory; Chrome and a WPT checkout are only needed to (re)record them.

- `fixtures/<wpt path>/<page>.<block>.txt`: one fixture per block of inline text on a
  WPT page: the block's computed styles, its text runs (one per DOM text node, `<br>`
  as a newline) with the styles that differ from the block's, and the rect Chrome
  reports for every code point (`Range.getClientRects()`, relative to the block's
  content box, in CSS px).
- `fonts/`: the `@font-face` fonts the fixtures use, copied from the WPT checkout.
  Blocks that would fall back to a system font are not recorded, so the fixtures
  are deterministic.
- `expectations.txt`: fixtures that currently `FAIL` (Parley places some character
  elsewhere than Chrome, within the tolerances in `parley_wpt_cases::compare`) or
  `ERROR` (the fixture cannot be laid out). Everything else must pass. Fixtures using
  CSS features Parley has no notion of (`parley_wpt_cases::check_supported`) are
  skipped and not listed.

## Running

```sh
cargo test -p parley_tests wpt
# Only fixtures whose path contains a string, printing their diffs:
PARLEY_WPT_FILTER=white-space/pre-wrap cargo test -p parley_tests wpt
# Rewrite expectations.txt from the actual outcomes:
PARLEY_TEST=accept cargo test -p parley_tests wpt
```

The test fails when a fixture's outcome differs from `expectations.txt` in either
direction: a change that fixes fixtures has to be accepted too, so the expectations
document what Parley gets right.

## Recording

Needs a WPT checkout, Chrome (for Testing) and a matching chromedriver:

```sh
git clone --depth 1 https://github.com/web-platform-tests/wpt ../wpt
npx @puppeteer/browsers install chrome@stable
npx @puppeteer/browsers install chromedriver@stable

PARLEY_WPT_CHROME=/path/to/chrome PARLEY_WPT_CHROMEDRIVER=/path/to/chromedriver \
  cargo run -p parley_wpt_recorder -- \
    css/css-text css/CSS2/text css/CSS2/bidi-text css/css-inline css/css-writing-modes
PARLEY_TEST=accept cargo test -p parley_tests wpt
```

`--wpt-dir` (or `WPT_DIR`) points at the checkout when it is not `../wpt`; the
binaries default to `google-chrome`/`chromedriver` on `PATH`. Paths are directories or
pages inside the checkout; the fixtures under each path are removed and rewritten,
so a path can be re-recorded on its own. The recorder prints every block it does not
record and why (system fonts, block-level or replaced children, floats, vertical
writing modes, `::first-line`, ...). Recording the five suites above takes a few
minutes and is deterministic for a given Chrome version; note the version in the
commit message when re-recording.
