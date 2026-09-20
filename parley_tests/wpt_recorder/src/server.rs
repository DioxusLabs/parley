// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A minimal static file server for the WPT checkout.
//!
//! WPT pages reference shared resources by absolute path (`/fonts/ahem.css`,
//! `/resources/testharness.js`), so they must be served from the checkout root rather
//! than opened via `file://`. This serves `GET` requests for files under that root, one
//! connection per thread, and nothing else.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};

/// A running server. Its threads exit with the process.
pub(crate) struct Server {
    port: u16,
}

impl Server {
    /// Serves `root` on a free loopback port.
    pub(crate) fn start(root: PathBuf) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let root = root.clone();
                std::thread::spawn(move || {
                    let _ = handle(stream, &root);
                });
            }
        });
        Ok(Self { port })
    }

    /// The URL of `path` (relative to the root) on this server.
    #[must_use]
    pub(crate) fn url(&self, path: &str) -> String {
        format!(
            "http://127.0.0.1:{}/{}",
            self.port,
            path.trim_start_matches('/')
        )
    }
}

fn handle(mut stream: TcpStream, root: &Path) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    // Drain the headers; nothing in them changes the response.
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }

    let mut parts = request_line.split_whitespace();
    let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else {
        return respond(&mut stream, 405, "text/plain", b"method not allowed");
    };
    let path = target.split(['?', '#']).next().unwrap_or("");
    let Some(file) = resolve(root, path) else {
        return respond(&mut stream, 404, "text/plain", b"not found");
    };
    let mut body = Vec::new();
    match std::fs::File::open(&file).and_then(|mut f| f.read_to_end(&mut body)) {
        Ok(_) => respond(&mut stream, 200, content_type(&file), &body),
        Err(_) => respond(&mut stream, 404, "text/plain", b"not found"),
    }
}

/// Maps a request path to a file under `root`, refusing anything that escapes it.
fn resolve(root: &Path, path: &str) -> Option<PathBuf> {
    let decoded = percent_decode(path);
    let mut file = root.to_path_buf();
    for component in Path::new(&decoded).components() {
        match component {
            Component::Normal(part) => file.push(part),
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    if file.is_dir() {
        file.push("index.html");
    }
    file.is_file().then_some(file)
}

fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(Ok(byte)) = path
                .get(i + 1..i + 3)
                .map(|hex| u8::from_str_radix(hex, 16))
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn content_type(file: &Path) -> &'static str {
    match file.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "xht" | "xhtml" => "application/xhtml+xml; charset=utf-8",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Method Not Allowed",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}
