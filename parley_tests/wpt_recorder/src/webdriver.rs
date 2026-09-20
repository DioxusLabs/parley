// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Running chromedriver and a headless Chrome session.
//!
//! The binaries come from the environment: `PARLEY_WPT_CHROMEDRIVER` (default:
//! `chromedriver` on `PATH`) and `PARLEY_WPT_CHROME` (default: whatever chromedriver
//! finds). Use a Chrome for Testing build matching the chromedriver version.

use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use fantoccini::wd::TimeoutConfiguration;
use fantoccini::{Client, ClientBuilder};
use hyper_util::client::legacy::connect::HttpConnector;
use tokio::time::timeout;

/// How long to wait for any single step before giving up.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(20);

/// WPT reftests are written for an 800×600 viewport.
pub(crate) const VIEWPORT: (u32, u32) = (800, 600);

/// Chrome launch flags.
///
/// `--font-render-hinting=none` stops headless Chrome quantising glyph advances to
/// whole pixels; `--force-device-scale-factor=1` and `--hide-scrollbars` keep CSS px
/// equal to device px and stop scrollbars stealing width.
const CHROME_ARGS: &[&str] = &[
    "--headless=new",
    "--no-sandbox",
    "--disable-gpu",
    "--font-render-hinting=none",
    "--force-device-scale-factor=1",
    "--hide-scrollbars",
    "--disable-dev-shm-usage",
    "--lang=en-US",
];

/// A running chromedriver process.
pub(crate) struct WebDriver {
    process: Child,
    url: String,
    profile_dir: PathBuf,
}

impl WebDriver {
    /// Starts chromedriver on a free port.
    pub(crate) fn start() -> Result<Self, String> {
        let chromedriver =
            std::env::var("PARLEY_WPT_CHROMEDRIVER").unwrap_or_else(|_| "chromedriver".into());
        let port = free_port().map_err(|error| format!("finding a free port: {error}"))?;
        let mut process = Command::new(&chromedriver)
            .arg(format!("--port={port}"))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("starting {chromedriver}: {error}"))?;

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let start = Instant::now();
        loop {
            if let Ok(Some(status)) = process.try_wait() {
                return Err(format!("chromedriver exited during startup ({status})"));
            }
            if TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok() {
                break;
            }
            if start.elapsed() > TIMEOUT {
                let _ = process.kill();
                return Err("chromedriver did not start listening".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let profile_dir =
            std::env::temp_dir().join(format!("parley-wpt-record-{}", std::process::id()));
        Ok(Self {
            process,
            url: format!("http://127.0.0.1:{port}"),
            profile_dir,
        })
    }

    /// Opens a headless Chrome session.
    pub(crate) async fn new_session(&self) -> Result<Client, String> {
        let mut args: Vec<String> = CHROME_ARGS.iter().map(ToString::to_string).collect();
        args.push(format!("--user-data-dir={}", self.profile_dir.display()));
        args.push(format!("--window-size={},{}", VIEWPORT.0, VIEWPORT.1));
        let mut options = serde_json::json!({ "args": args });
        if let Ok(binary) = std::env::var("PARLEY_WPT_CHROME") {
            options["binary"] = serde_json::Value::String(binary);
        }
        let mut capabilities = serde_json::Map::new();
        capabilities.insert("goog:chromeOptions".into(), options);

        let mut builder = ClientBuilder::new(HttpConnector::new());
        builder.capabilities(capabilities);
        let client = timeout(TIMEOUT, builder.connect(&self.url))
            .await
            .map_err(|_| "the browser did not start in time".to_string())?
            .map_err(|error| format!("creating a WebDriver session: {error}"))?;
        client
            .update_timeouts(TimeoutConfiguration::new(
                Some(TIMEOUT),
                Some(TIMEOUT),
                Some(Duration::ZERO),
            ))
            .await
            .map_err(|error| format!("setting WebDriver timeouts: {error}"))?;
        client
            .set_window_size(VIEWPORT.0, VIEWPORT.1)
            .await
            .map_err(|error| format!("setting the window size: {error}"))?;
        Ok(client)
    }

    /// Closes the session and stops chromedriver.
    pub(crate) async fn shutdown(self, client: Client) {
        let _ = timeout(TIMEOUT, client.close()).await;
        self.shutdown_without_session();
    }

    /// Stops chromedriver when no session was opened.
    pub(crate) fn shutdown_without_session(mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
        let _ = std::fs::remove_dir_all(&self.profile_dir);
    }
}

fn free_port() -> std::io::Result<u16> {
    Ok(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?
        .local_addr()?
        .port())
}
