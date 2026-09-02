//! The OOXML package: ZIP-level access, and the byte-preserving rewrite path.
//!
//! ## Why raw copy, and not read-then-write
//!
//! A repair changes a handful of attributes in one or two parts. Every other
//! part must come out exactly as it went in — and "exactly" here means at the
//! byte level, because that is the only claim that can be *tested*.
//!
//! Decompressing an entry and compressing it again does not achieve that, even
//! when the bytes in between are identical. The deflate stream depends on the
//! encoder and its level, so a package written by PowerPoint at one setting and
//! re-emitted by us at another differs in every compressed entry while the
//! documents remain semantically equal. That is a difference no reviewer can
//! check and no test can usefully assert on.
//!
//! So untouched entries are copied *raw*: the already-compressed bytes move
//! across verbatim, along with the entry's CRC, sizes, compression method,
//! timestamp and permissions. Only a part carrying an edit is decompressed,
//! rewritten and compressed again — and that part was going to change anyway.
//!
//! This is invariant 3 in `AGENTS.md` made mechanical rather than aspirational.

use mirsam_core::error::{Error, Result};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use zip::{ZipArchive, ZipWriter};

/// Parts to replace on rewrite, keyed by package-relative name.
///
/// A `BTreeMap` rather than a `HashMap` so a rewrite is deterministic: two runs
/// over the same edits produce the same file.
pub type Edits = BTreeMap<String, Vec<u8>>;

fn zip_err(part: &str, e: zip::result::ZipError) -> Error {
    Error::Format(format!("{part}: {e}"))
}

/// True when both paths name the same existing file.
///
/// Falls back to a literal comparison when either side cannot be canonicalised
/// — which is the normal case for an output path that does not exist yet, and
/// is safe: a file that does not exist cannot be the source.
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// An OOXML package on disk.
///
/// Holds the path rather than the archive: an audit reads the package once and
/// a repair reads it again while writing. Keeping a `ZipArchive` alive across
/// both would mean either a mutable borrow that outlives its usefulness or a
/// full in-memory copy of a file that can be tens of megabytes.
pub struct Package {
    path: PathBuf,
}

impl Package {
    /// Open a package, verifying it is a readable OOXML container.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if !path.exists() {
            return Err(Error::NotFound);
        }
        let pkg = Self { path };
        let mut archive = pkg.archive()?;
        if archive.by_name("[Content_Types].xml").is_err() {
            return Err(Error::Format(
                "not an OOXML package: [Content_Types].xml is missing".into(),
            ));
        }
        Ok(pkg)
    }

    /// The path this package was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn archive(&self) -> Result<ZipArchive<BufReader<File>>> {
        let file = File::open(&self.path)?;
        ZipArchive::new(BufReader::new(file))
            .map_err(|e| Error::Format(format!("not a readable OOXML package: {e}")))
    }

    /// Every entry name, in the order the package stores them.
    ///
    /// Order is preserved rather than sorted because it is part of what a
    /// rewrite must reproduce.
    pub fn part_names(&self) -> Result<Vec<String>> {
        Ok(self.archive()?.file_names().map(str::to_string).collect())
    }

    /// Entry names matching a predicate, in package order.
    pub fn parts_where(&self, f: impl Fn(&str) -> bool) -> Result<Vec<String>> {
        Ok(self
            .archive()?
            .file_names()
            .filter(|n| f(n))
            .map(str::to_string)
            .collect())
    }

    /// Read one part as UTF-8 text.
    pub fn read_text(&self, part: &str) -> Result<String> {
        let mut archive = self.archive()?;
        let mut buf = String::new();
        archive
            .by_name(part)
            .map_err(|e| zip_err(part, e))?
            .read_to_string(&mut buf)?;
        Ok(buf)
    }

    /// Read one part as raw bytes.
    pub fn read_bytes(&self, part: &str) -> Result<Vec<u8>> {
        let mut archive = self.archive()?;
        let mut buf = Vec::new();
        archive
            .by_name(part)
            .map_err(|e| zip_err(part, e))?
            .read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Write the package to `dest`, replacing the parts named in `edits` and
    /// copying every other entry raw.
    ///
    /// Returns the number of parts actually replaced. An edit naming a part the
    /// package does not contain is an error rather than a silent no-op: it means
    /// the caller's idea of the document and the document itself disagree.
    ///
    /// Refuses to write over its own source. The original is always preserved.
    pub fn rewrite(&self, dest: &Path, edits: &Edits) -> Result<usize> {
        if same_file(&self.path, dest) {
            return Err(Error::WouldOverwriteSource);
        }

        let mut archive = self.archive()?;
        let known: Vec<String> = archive.file_names().map(str::to_string).collect();
        if let Some(missing) = edits.keys().find(|k| !known.contains(k)) {
            return Err(Error::Format(format!(
                "cannot rewrite {missing:?}: no such part in {}",
                self.path.display()
            )));
        }

        // Write to a sibling temporary and rename into place, so an interrupted
        // rewrite cannot leave a half-written document where a whole one was.
        let tmp = temp_sibling(dest);
        let mut applied = 0usize;
        {
            let out = File::create(&tmp)?;
            let mut writer = ZipWriter::new(BufWriter::new(out));

            for i in 0..archive.len() {
                let entry = archive.by_index_raw(i).map_err(|e| zip_err("<entry>", e))?;
                let name = entry.name().to_string();

                match edits.get(&name) {
                    // Edited: inherit the original entry's compression method,
                    // timestamp and permissions, and supply new content.
                    Some(bytes) => {
                        let options = entry.options();
                        drop(entry);
                        writer
                            .start_file(&name, options)
                            .map_err(|e| zip_err(&name, e))?;
                        writer.write_all(bytes)?;
                        applied += 1;
                    }
                    // Untouched: move the compressed bytes across verbatim.
                    None => writer.raw_copy_file(entry).map_err(|e| zip_err(&name, e))?,
                }
            }

            writer
                .finish()
                .map_err(|e| zip_err("<central directory>", e))?
                .flush()?;
        }

        fs::rename(&tmp, dest).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })?;
        Ok(applied)
    }
}

/// A temporary path beside `dest`, so the rename that follows stays on one
/// filesystem and is therefore atomic.
fn temp_sibling(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".mirsam-{}.tmp", std::process::id()));
    dest.with_file_name(name)
}
