// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Persistent cache for file system font scans.
//!
//! The cache stores, for every scanned file, the file's modification time
//! and size along with the font metadata extracted from it. On subsequent
//! scans, files whose modification time and size are unchanged can be
//! loaded from the cache instead of being read and parsed again.
//!
//! The format is a simple little-endian binary encoding with a magic number
//! and version. Any file that fails to decode (wrong magic, wrong version,
//! truncated data, etc.) is ignored, causing a full rescan that rewrites
//! the cache.

#![cfg(feature = "std")]

use super::font::{AxisInfo, AxisVec, FontInfo};
use super::scan::FontRecord;
use super::source::{SourceId, SourceInfo, SourceKind};
use crate::{CharmapIndex, FontStyle, FontWeight, FontWidth};
use alloc::string::String;
use alloc::vec::Vec;
use hashbrown::HashMap;
use read_fonts::types::Tag;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

const MAGIC: &[u8; 4] = b"fqsc";
const VERSION: u16 = 1;

/// Modification time and size of a file, used to detect changes.
#[derive(Copy, Clone, PartialEq, Eq, Default)]
pub(crate) struct FileStamp {
    mtime_secs: u64,
    mtime_nanos: u32,
    size: u64,
}

impl FileStamp {
    pub(crate) fn from_metadata(metadata: &std::fs::Metadata) -> Option<Self> {
        let mtime = metadata
            .modified()
            .ok()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?;
        Some(Self {
            mtime_secs: mtime.as_secs(),
            mtime_nanos: mtime.subsec_nanos(),
            size: metadata.len(),
        })
    }
}

/// Cached scan results for a single file.
pub(crate) struct CachedFile {
    pub(crate) stamp: FileStamp,
    pub(crate) records: Vec<FontRecord>,
}

/// Loads the cache from the given path.
///
/// Returns `None` if the cache doesn't exist or can't be decoded.
pub(crate) fn load(path: &Path) -> Option<HashMap<PathBuf, CachedFile>> {
    let data = std::fs::read(path).ok()?;
    let mut reader = Reader { data: &data };
    if reader.bytes(4)? != MAGIC || reader.u16()? != VERSION {
        return None;
    }
    let file_count = reader.u32()?;
    let mut files = HashMap::with_capacity(file_count as usize);
    for _ in 0..file_count {
        let path = PathBuf::from(reader.str()?);
        let stamp = FileStamp {
            mtime_secs: reader.u64()?,
            mtime_nanos: reader.u32()?,
            size: reader.u64()?,
        };
        let record_count = reader.u32()?;
        let mut records = Vec::with_capacity(record_count.min(64) as usize);
        let source: Arc<Path> = path.as_path().into();
        for _ in 0..record_count {
            records.push(read_record(&mut reader, &source)?);
        }
        files.insert(path, CachedFile { stamp, records });
    }
    Some(files)
}

/// Saves the cache to the given path.
///
/// The cache is written to a temporary file first and then renamed into
/// place so that concurrent readers never observe a partially written
/// cache.
pub(crate) fn save<'a>(
    path: &Path,
    files: impl Iterator<Item = (&'a Path, FileStamp, &'a [FontRecord])>,
) -> Option<()> {
    let mut writer = Writer::default();
    writer.bytes(MAGIC);
    writer.u16(VERSION);
    let file_count_position = writer.data.len();
    writer.u32(0);
    let mut file_count: u32 = 0;
    for (file_path, stamp, records) in files {
        let Some(path_str) = file_path.to_str() else {
            continue;
        };
        let Ok(record_count) = u32::try_from(records.len()) else {
            continue;
        };
        writer.str(path_str);
        writer.u64(stamp.mtime_secs);
        writer.u32(stamp.mtime_nanos);
        writer.u64(stamp.size);
        writer.u32(record_count);
        for record in records {
            write_record(&mut writer, record);
        }
        file_count = file_count.checked_add(1)?;
    }
    writer.data[file_count_position..file_count_position + 4]
        .copy_from_slice(&file_count.to_le_bytes());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    let temp_path = path.with_extension(alloc::format!("tmp-{}", std::process::id()));
    std::fs::write(&temp_path, &writer.data).ok()?;
    std::fs::rename(&temp_path, path).ok()
}

fn read_record(reader: &mut Reader<'_>, source_path: &Arc<Path>) -> Option<FontRecord> {
    let index = reader.u32()?;
    let name_count = reader.u16()?;
    let mut names = Vec::with_capacity(usize::from(name_count.min(16)));
    for _ in 0..name_count {
        names.push(String::from(reader.str()?));
    }
    let postscript_name = String::from(reader.str()?);
    let width = FontWidth::from_ratio(reader.f32()?);
    let style = match reader.u8()? {
        0 => FontStyle::Normal,
        1 => FontStyle::Italic,
        2 => FontStyle::Oblique(None),
        3 => FontStyle::Oblique(Some(reader.f32()?)),
        _ => return None,
    };
    let weight = FontWeight::new(reader.f32()?);
    let axis_count = reader.u16()?;
    let mut axes = AxisVec::with_capacity(usize::from(axis_count.min(64)));
    for _ in 0..axis_count {
        axes.push(AxisInfo {
            tag: Tag::new(reader.bytes(4)?.try_into().ok()?),
            min: reader.f32()?,
            max: reader.f32()?,
            default: reader.f32()?,
        });
    }
    let charmap_index =
        CharmapIndex::from_parts((reader.u32()?, reader.u8()? != 0, reader.u8()? != 0));
    let source = SourceInfo::new(SourceId::new(), SourceKind::Path(source_path.clone()));
    Some(FontRecord {
        names,
        postscript_name,
        font: FontInfo::from_parts(source, index, width, style, weight, axes, charmap_index),
    })
}

fn write_record(writer: &mut Writer, record: &FontRecord) {
    let font = &record.font;
    writer.u32(font.index());
    writer.u16(record.names.len().min(u16::MAX as usize) as u16);
    for name in record.names.iter().take(u16::MAX as usize) {
        writer.str(name);
    }
    writer.str(&record.postscript_name);
    writer.f32(font.width().ratio());
    match font.style() {
        FontStyle::Normal => writer.u8(0),
        FontStyle::Italic => writer.u8(1),
        FontStyle::Oblique(None) => writer.u8(2),
        FontStyle::Oblique(Some(angle)) => {
            writer.u8(3);
            writer.f32(angle);
        }
    }
    writer.f32(font.weight().value());
    writer.u16(font.axes().len().min(u16::MAX as usize) as u16);
    for axis in font.axes().iter().take(u16::MAX as usize) {
        writer.bytes(&axis.tag.to_be_bytes());
        writer.f32(axis.min);
        writer.f32(axis.max);
        writer.f32(axis.default);
    }
    let (subtable_offset, is_symbol, is_mac_roman) = font.charmap_index().to_parts();
    writer.u32(subtable_offset);
    writer.u8(u8::from(is_symbol));
    writer.u8(u8::from(is_mac_roman));
}

struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let (bytes, rest) = self.data.split_at_checked(len)?;
        self.data = rest;
        Some(bytes)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.bytes(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.bytes(8)?.try_into().ok()?))
    }

    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }

    fn str(&mut self) -> Option<&'a str> {
        let len = self.u32()?;
        core::str::from_utf8(self.bytes(usize::try_from(len).ok()?)?).ok()
    }
}

#[derive(Default)]
struct Writer {
    data: Vec<u8>,
}

impl Writer {
    fn bytes(&mut self, bytes: &[u8]) {
        self.data.extend_from_slice(bytes);
    }

    fn u8(&mut self, value: u8) {
        self.data.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    fn f32(&mut self, value: f32) {
        self.bytes(&value.to_le_bytes());
    }

    fn str(&mut self, value: &str) {
        self.u32(value.len().min(u32::MAX as usize) as u32);
        self.bytes(&value.as_bytes()[..value.len().min(u32::MAX as usize)]);
    }
}
