# Benchmarking Parley

Practices for timing and profiling Parley itself, distilled from a full-history
performance sweep and per-PR profiling work.

## Measure phases separately

A layout has four independently-timed phases; regressions hide in composites:

1. **build** — `RangedBuilder` + `layout.build(&mut lcx)` (itemize, `CharInfo`,
   font selection, shaping). The heaviest phase (~40–55 ms on 250 KB prose).
2. **break+align** — `layout.break_all_lines(Some(w))` + `layout.align(...)`.
   Hot loop: per-atom iteration in `break_next_line_or_box`.
3. **content widths** — `layout.calculate_content_widths()` (`Atoms::next` scan).
4. **glyph iteration** — walking `layout.lines()` → positioned items/glyphs.

## Methodology

- **Use document-scale input.** ~250 KB of real prose (e.g. a Wikipedia article)
  at a realistic wrap width (800 px). Excerpt-sized inputs (≤8 KB) miss
  superlinear behavior and have a noise floor near the code under test.
- **Interleave A/B runs.** Alternate the two binaries across ≥3 passes, n≥30
  per pass. Report per-phase **minimums** — the most stable statistic on a
  shared/noisy box.
- **Pin cores** (`taskset -c 0-3` on Linux) and build both binaries in the same
  directory profile.
- **Verify output before trusting timing.** Layout output (line count,
  `ContentWidths`, a glyph checksum) must be byte-identical; a "win" that
  changes output is a bug.
- **Never compare absolutes across days.** The same binary can drift ±15%
  between machine days. Re-measure your anchor commit in the same session, and
  mark measurement-day boundaries on long-lived graphs.
- **Distrust small deltas from a single build pair.** Same-source builds differ
  ±3–10% from codegen layout luck — unrelated diffs flip µop-cache (DSB) vs
  legacy-decode (MITE) behavior on hot loops. Below ~2–3% the effect may not be
  real. `-C llvm-args=-align-all-functions=6` collapses most of this spread for
  comparison builds; `lto="fat"` + `codegen-units=1` and PGO give real wins
  but do not remove layout sensitivity on their own.

## Profiling pitfalls

- **Inlining collapses attribution.** A leaf symbol showing "90% self time" may
  have the whole hot loop folded under it (observed: `update_max_height_exceeded`
  absorbing all of `break_next_line_or_box`). Cross-check with inclusive time
  and deletion experiments before attributing a hotspot.
- **Counters expose what timing hides.** Frontend-decode counters
  (`idq.mite_uops`, `dsb2mite_switches.penalty_cycles`) are what reveal the
  codegen-layout effect described above; instruction counts and branch misses
  stay flat while they move.
- **Corpus matters for fallback paths.** Font-fallback cost is per-cluster and
  only shows on text that actually misses primary coverage — multi-script,
  emoji-heavy, or uncovered-codepoint inputs. Clean prose profiles near zero.

## Structural notes (where time goes on current main)

- `break_all_lines` clones `LineState` at every soft-break opportunity (spaces,
  `OverflowWrap` points). Keep `LineState`/`LineBoxMetrics` `Copy`-small — any
  heap field (e.g. `SmallVec`) is deep-copied per opportunity and dominated
  break-phase samples in practice. Prefer flat buffers on the breaker state
  with length-based snapshots (`truncate` on revert).
- `calculate_content_widths` probes each atom's first `Character` for
  whitespace/boundary/style — caching those flags on `ShapedCluster` avoids
  re-deref through the character array.
- `build` time spreads across itemization, per-char `CharInfo` property bits,
  `select_font`, and ICU4X segmentation — usually no single dominant leaf; treat
  "one big hotspot" claims with suspicion and check whether the diff is really
  diffuse per-char work or codegen luck.
