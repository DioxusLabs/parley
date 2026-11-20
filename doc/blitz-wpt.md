# Blitz Web Platform Tests

The **Blitz WPT** check runs the `css` and `svg` Web Platform Tests with Parley from the PR head and its merge-base. Both runs use the same resolved Blitz commit and WPT revision, with the local Parley workspace substituted for Blitz's Parley dependency. A companion Blitz pin PR is not needed.

The workflow summary and the generated section of the PR description show the diff, per-area changes and tested commit hashes. The run artifacts include compressed base/candidate reports and a `wpt-diff` artifact with the full comparison. Existing WPT failures do not fail the CI check; build and runner errors do.

## API-breaking changes

Push the required Blitz adapter changes to a branch in `DioxusLabs/blitz` and add a standalone line to the Parley PR description:

```text
blitz-revision: <full 40-character Blitz commit SHA>
```

Only the candidate uses this Blitz commit. The baseline uses Blitz main, and the report flags that the diff includes Blitz compatibility changes. A Blitz PR is not required just to run WPT.

Adding, changing or removing the field reruns the comparison. Other description edits do not. Update the field after pushing new adapter commits; branch names and abbreviated SHAs are rejected. Stale results are not published after the Parley head or selected override changes. **Re-run all jobs** in Actions retries a run without editing the PR.

## CI maintenance

The reusable workflow and its helper scripts are pinned to a Blitz commit in the two WPT workflow files. Update those pins together when upgrading the tooling. Tests run with read-only permissions; a separate trusted workflow publishes the result section and never executes PR code or artifact scripts.

The DioxusLabs fork uses WarpBuild runners. Other callers use standard GitHub-hosted runners and GitHub's cache provider, so the workflow does not require access to DioxusLabs runners.
