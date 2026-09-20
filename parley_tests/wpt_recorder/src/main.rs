// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Records WPT text-layout fixtures from Chrome.
//!
//! ```text
//! cargo run -p parley_wpt_recorder -- [--wpt-dir DIR] [--out DIR] PATH...
//! ```
//!
//! Each `PATH` is a file or directory inside the WPT checkout (`--wpt-dir`, or
//! `$WPT_DIR`, or `../wpt` next to this repository), e.g.
//! `css/css-text/white-space`. Every `.html`/`.xht`/`.xhtml` page under it is loaded in
//! headless Chrome (via chromedriver, see `webdriver.rs`) and each block of plain inline
//! text on it is written as a fixture to `<out>/fixtures/<path>/<page>.<block>.txt`,
//! with the fonts it needs copied to `<out>/fonts/`. Existing fixtures under a given
//! `PATH` are removed first so renamed or removed WPT tests do not leave stale files.
//!
//! `--out` defaults to `parley_tests/tests/wpt` in this repository.

mod extract;
mod server;
mod webdriver;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fantoccini::Client;
use tokio::time::timeout;
use walkdir::WalkDir;

use extract::{FontStore, Page};
use server::Server;
use webdriver::{TIMEOUT, WebDriver};

const EXTRACT_JS: &str = include_str!("extract.js");

/// Waits for fonts and the load event so the layout Chrome reports is final.
const READY_JS: &str = r#"
const done = arguments[0];
const settle = () => document.fonts.ready.then(() => requestAnimationFrame(() => done(true)));
if (document.readyState === "complete") { settle(); } else { addEventListener("load", settle); }
"#;

struct Options {
    wpt_dir: PathBuf,
    out_dir: PathBuf,
    paths: Vec<String>,
}

fn usage() -> ! {
    eprintln!("usage: wpt_record [--wpt-dir DIR] [--out DIR] PATH...");
    std::process::exit(2);
}

fn parse_args() -> Options {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut wpt_dir = std::env::var_os("WPT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root.join("../wpt"));
    let mut out_dir = repo_root.join("parley_tests/tests/wpt");
    let mut paths = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--wpt-dir" => wpt_dir = args.next().map(PathBuf::from).unwrap_or_else(|| usage()),
            "--out" => out_dir = args.next().map(PathBuf::from).unwrap_or_else(|| usage()),
            "-h" | "--help" => usage(),
            _ if arg.starts_with('-') => usage(),
            _ => paths.push(arg.trim_matches('/').to_string()),
        }
    }
    if paths.is_empty() {
        usage();
    }
    Options {
        wpt_dir,
        out_dir,
        paths,
    }
}

fn is_test_page(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    if !matches!(extension, "html" | "xht" | "xhtml") {
        return false;
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    // Manual tests need a human; crash tests exist to not crash.
    !stem.ends_with("-manual") && !stem.ends_with("-crash")
}

/// Pages to record under `path` (relative to the WPT checkout), sorted.
fn pages(wpt_dir: &Path, path: &str) -> Vec<String> {
    let root = wpt_dir.join(path);
    if root.is_file() {
        return vec![path.to_string()];
    }
    let mut pages: Vec<String> = WalkDir::new(&root)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !(entry.file_type().is_dir()
                && matches!(
                    name.as_ref(),
                    "support" | "resources" | "tools" | "crashtests"
                ))
        })
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && is_test_page(entry.path()))
        .filter_map(|entry| {
            entry
                .path()
                .strip_prefix(wpt_dir)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    pages.sort();
    pages
}

async fn record_page(client: &Client, url: &str) -> Result<Page, String> {
    timeout(TIMEOUT, client.goto(url))
        .await
        .map_err(|_| "timed out loading".to_string())?
        .map_err(|error| format!("loading: {error}"))?;
    timeout(TIMEOUT, client.execute_async(READY_JS, vec![]))
        .await
        .map_err(|_| "timed out waiting for fonts".to_string())?
        .map_err(|error| format!("waiting for fonts: {error}"))?;
    let json = timeout(TIMEOUT, client.execute(EXTRACT_JS, vec![]))
        .await
        .map_err(|_| "timed out extracting".to_string())?
        .map_err(|error| format!("extracting: {error}"))?;
    let json = json
        .as_str()
        .ok_or_else(|| "the extraction script did not return a string".to_string())?;
    serde_json::from_str(json).map_err(|error| format!("parsing the extraction: {error}"))
}

fn fixture_dir(out_dir: &Path, page: &str) -> PathBuf {
    let parent = Path::new(page).parent().unwrap_or(Path::new(""));
    out_dir.join("fixtures").join(parent)
}

fn fixture_name(page: &str, block: usize) -> String {
    let stem = Path::new(page)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("page");
    format!("{stem}.{block}.txt")
}

async fn run(options: Options) -> Result<(), String> {
    if !options.wpt_dir.is_dir() {
        return Err(format!(
            "{} is not a directory; pass --wpt-dir or set WPT_DIR to a WPT checkout",
            options.wpt_dir.display()
        ));
    }
    let wpt_dir = options
        .wpt_dir
        .canonicalize()
        .map_err(|error| format!("{}: {error}", options.wpt_dir.display()))?;
    let out_dir = &options.out_dir;
    let mut fonts = FontStore::new(&wpt_dir, &out_dir.join("fonts"))
        .map_err(|error| format!("preparing {}: {error}", out_dir.display()))?;

    let server = Server::start(wpt_dir.clone()).map_err(|error| format!("serving WPT: {error}"))?;
    let driver = WebDriver::start()?;
    let client = match driver.new_session().await {
        Ok(client) => client,
        Err(error) => {
            driver.shutdown_without_session();
            return Err(error);
        }
    };

    let mut written = 0_usize;
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let mut failed = Vec::new();
    let result = async {
        for path in &options.paths {
            let stale = out_dir.join("fixtures").join(path);
            if stale.is_dir() {
                std::fs::remove_dir_all(&stale)
                    .map_err(|error| format!("clearing {}: {error}", stale.display()))?;
            } else if wpt_dir.join(path).is_file() {
                let dir = fixture_dir(out_dir, path);
                let prefix = format!(
                    "{}.",
                    Path::new(path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                );
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        if entry.file_name().to_string_lossy().starts_with(&prefix) {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
            }

            let pages = pages(&wpt_dir, path);
            if pages.is_empty() {
                return Err(format!("no test pages under {path}"));
            }
            for page in &pages {
                let recorded = record_page(&client, &server.url(page)).await;
                let recorded = match recorded {
                    Ok(recorded) => recorded,
                    Err(error) => {
                        eprintln!("{page}: {error}");
                        failed.push(page.clone());
                        continue;
                    }
                };
                let (cases, page_skipped) = extract::cases(page, &recorded, &mut fonts);
                for skip in page_skipped {
                    let reason = skip
                        .reason
                        .split_once(':')
                        .map_or(skip.reason.as_str(), |(head, _)| head)
                        .to_string();
                    *skipped.entry(reason).or_default() += 1;
                    eprintln!(
                        "{page} block {} ({}): {}",
                        skip.block, skip.path, skip.reason
                    );
                }
                let dir = fixture_dir(out_dir, page);
                std::fs::create_dir_all(&dir)
                    .map_err(|error| format!("creating {}: {error}", dir.display()))?;
                for case in cases {
                    let file = dir.join(fixture_name(page, case.block));
                    std::fs::write(&file, case.write())
                        .map_err(|error| format!("writing {}: {error}", file.display()))?;
                    written += 1;
                }
            }
        }
        Ok(())
    }
    .await;
    driver.shutdown(client).await;
    result?;
    let removed_fonts = remove_unused_fonts(out_dir)?;

    eprintln!(
        "\nwrote {written} fixtures to {}",
        out_dir.join("fixtures").display()
    );
    if removed_fonts > 0 {
        eprintln!("removed {removed_fonts} fonts no fixture uses");
    }
    if !skipped.is_empty() {
        eprintln!("skipped blocks:");
        let mut reasons: Vec<_> = skipped.into_iter().collect();
        reasons.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (reason, count) in reasons {
            eprintln!("  {count:5}  {reason}");
        }
    }
    if !failed.is_empty() {
        return Err(format!("{} pages failed to record", failed.len()));
    }
    Ok(())
}

/// Deletes fonts under `<out>/fonts` that no fixture under `<out>/fixtures` refers to
/// any more, returning how many.
fn remove_unused_fonts(out_dir: &Path) -> Result<usize, String> {
    let mut used = std::collections::BTreeSet::new();
    for entry in WalkDir::new(out_dir.join("fixtures"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let text = std::fs::read_to_string(entry.path())
            .map_err(|error| format!("reading {}: {error}", entry.path().display()))?;
        let case = parley_wpt_cases::Case::parse(&text)
            .map_err(|error| format!("{}: {error}", entry.path().display()))?;
        used.extend(case.fonts.into_iter().map(|font| font.file));
    }
    let mut removed = 0;
    for entry in std::fs::read_dir(out_dir.join("fonts"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().is_ok_and(|kind| kind.is_file()) && !used.contains(&name) {
            std::fs::remove_file(entry.path())
                .map_err(|error| format!("removing {}: {error}", entry.path().display()))?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn main() -> ExitCode {
    let options = parse_args();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    match runtime.block_on(run(options)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
