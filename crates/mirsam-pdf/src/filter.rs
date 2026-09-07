//! Stream filters: §7.4, the half of it that carries text.
//!
//! ## Which filters are implemented, and why the rest are named rather than
//! failed
//!
//! `FlateDecode`, `LZWDecode`, `ASCIIHexDecode`, `ASCII85Decode` and
//! `RunLengthDecode` are general-purpose: a content stream, a `ToUnicode` CMap,
//! an object stream or a cross-reference stream may arrive through any of them,
//! and a document whose text is behind one is a document this tool must read.
//!
//! `DCTDecode`, `JPXDecode`, `CCITTFaxDecode` and `JBIG2Decode` are image
//! codecs. Decoding one produces a raster, and mirsam has said it will never
//! rasterise — so a stream behind one is not text this tool failed to read, it
//! is text that is not in the file at all. [`Decoded::Stopped`] carries the
//! filter's name so the document can say which, which is ADR 0009's shape: a
//! source the adapter could not read is part of the report, and a scanned page
//! must never come back as a page with no defects.
//!
//! `Crypt` is the same shape for a different reason — a document reaching this
//! module through it was not refused at [`crate::Pdf::open`], which would be a
//! bug rather than a file.
//!
//! ## A truncated stream keeps what it had
//!
//! Real files are truncated, and inflate reports that as an error after
//! producing perfectly good bytes. Discarding them would turn a document with
//! a damaged tail into a document with no text — the failure this project
//! treats as worse than a miss. So a decode that produced output keeps it, and
//! only one that produced nothing is an error.

use flate2::read::{DeflateDecoder, ZlibDecoder};
use mirsam_core::error::{Error, Result};
use std::io::Read;

use crate::object::{Dictionary, Name, Object};

/// What came out of a filter chain.
pub enum Decoded {
    /// Every filter in the chain was applied.
    Bytes(Vec<u8>),
    /// The chain reached a filter this crate does not decode. Named, never
    /// silently dropped.
    Stopped { filter: Name },
}

/// The filters that end a chain rather than continue it: an image codec
/// produces pixels, and mirsam does not rasterise.
fn is_image_codec(name: &Name) -> bool {
    ["DCTDecode", "JPXDecode", "CCITTFaxDecode", "JBIG2Decode"]
        .iter()
        .any(|codec| name.is(codec))
}

/// Run `data` through `filters` in order, with `parms[i]` the parameters of
/// `filters[i]`.
///
/// A missing entry in `parms` is an empty dictionary, which is what an absent
/// `/DecodeParms` means.
pub fn decode(data: &[u8], filters: &[Name], parms: &[Dictionary]) -> Result<Decoded> {
    let mut bytes = data.to_vec();
    let empty = Dictionary::new();
    for (index, filter) in filters.iter().enumerate() {
        let parms = parms.get(index).unwrap_or(&empty);
        bytes = match filter {
            f if f.is("FlateDecode") || f.is("Fl") => predict(inflate(&bytes)?, parms)?,
            f if f.is("LZWDecode") || f.is("LZW") => {
                let early = parms
                    .get("EarlyChange")
                    .and_then(Object::as_i64)
                    .unwrap_or(1);
                predict(lzw(&bytes, early != 0)?, parms)?
            }
            f if f.is("ASCIIHexDecode") || f.is("AHx") => ascii_hex(&bytes),
            f if f.is("ASCII85Decode") || f.is("A85") => ascii85(&bytes)?,
            f if f.is("RunLengthDecode") || f.is("RL") => run_length(&bytes),
            // §7.4.1: an identity filter, which some writers spell explicitly.
            f if f.is("Identity") => bytes,
            f if is_image_codec(f) || f.is("Crypt") => {
                return Ok(Decoded::Stopped {
                    filter: filter.clone(),
                });
            }
            _ => {
                return Ok(Decoded::Stopped {
                    filter: filter.clone(),
                });
            }
        };
    }
    Ok(Decoded::Bytes(bytes))
}

/// zlib, then raw deflate.
///
/// Both spellings are in the wild: §7.4.4 requires the zlib wrapper, and a
/// long tail of writers emits the bare deflate stream. Trying the wrapper
/// first keeps the conformant case exact.
fn inflate(data: &[u8]) -> Result<Vec<u8>> {
    let trimmed = {
        let start = data
            .iter()
            .position(|b| !crate::object::is_whitespace(*b))
            .unwrap_or(data.len());
        &data[start..]
    };

    let mut out = Vec::new();
    let zlib = ZlibDecoder::new(trimmed).read_to_end(&mut out);
    if zlib.is_ok() || !out.is_empty() {
        return Ok(out);
    }

    out.clear();
    let raw = DeflateDecoder::new(trimmed).read_to_end(&mut out);
    if raw.is_ok() || !out.is_empty() {
        return Ok(out);
    }
    Err(Error::Format(format!(
        "FlateDecode: {}",
        zlib.err()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "not a deflate stream".into())
    )))
}

/// §7.4.4.4's predictors, which a stream carries when it holds rows of equal
/// width — a cross-reference stream always does, and an image often does.
fn predict(data: Vec<u8>, parms: &Dictionary) -> Result<Vec<u8>> {
    let int = |key: &str, default: i64| parms.get(key).and_then(Object::as_i64).unwrap_or(default);
    let predictor = int("Predictor", 1);
    if predictor < 2 {
        return Ok(data);
    }
    let colors = int("Colors", 1).clamp(1, 32) as usize;
    let bpc = int("BitsPerComponent", 8).clamp(1, 32) as usize;
    let columns = int("Columns", 1).max(1) as usize;

    let bpp = (colors * bpc).div_ceil(8).max(1);
    let row = (colors * bpc * columns).div_ceil(8);

    if predictor == 2 {
        return Ok(tiff_predictor(data, colors, bpc, row));
    }

    // PNG: each row is preceded by its filter type.
    let mut out = Vec::with_capacity(data.len());
    let mut previous = vec![0u8; row];
    for chunk in data.chunks(row + 1) {
        // A truncated final row is dropped rather than un-filtered against
        // bytes that are not there.
        if chunk.len() < 2 {
            break;
        }
        let (kind, encoded) = (chunk[0], &chunk[1..]);
        let mut current = encoded.to_vec();
        for i in 0..current.len() {
            let left = if i >= bpp { current[i - bpp] } else { 0 };
            let up = *previous.get(i).unwrap_or(&0);
            let up_left = if i >= bpp {
                *previous.get(i - bpp).unwrap_or(&0)
            } else {
                0
            };
            current[i] = match kind {
                0 => current[i],
                1 => current[i].wrapping_add(left),
                2 => current[i].wrapping_add(up),
                3 => current[i].wrapping_add(((u16::from(left) + u16::from(up)) / 2) as u8),
                4 => current[i].wrapping_add(paeth(left, up, up_left)),
                other => {
                    return Err(Error::Format(format!(
                        "unknown PNG predictor filter type {other}"
                    )));
                }
            };
        }
        out.extend_from_slice(&current);
        previous = current;
        previous.resize(row, 0);
    }
    Ok(out)
}

fn tiff_predictor(mut data: Vec<u8>, colors: usize, bpc: usize, row: usize) -> Vec<u8> {
    // Only the byte-aligned case is undone. Sub-byte components appear in
    // images and never in a stream this crate reads for text, and guessing at
    // one would corrupt bytes rather than leave them alone.
    if bpc != 8 || row == 0 {
        return data;
    }
    for start in (0..data.len()).step_by(row) {
        let end = (start + row).min(data.len());
        for i in (start + colors)..end {
            data[i] = data[i].wrapping_add(data[i - colors]);
        }
    }
    data
}

fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    let p = i16::from(left) + i16::from(up) - i16::from(up_left);
    let (dl, du, dul) = (
        (p - i16::from(left)).abs(),
        (p - i16::from(up)).abs(),
        (p - i16::from(up_left)).abs(),
    );
    if dl <= du && dl <= dul {
        left
    } else if du <= dul {
        up
    } else {
        up_left
    }
}

/// §7.4.2. White space is ignored, `>` ends the data, and an odd final digit
/// is padded with a zero.
fn ascii_hex(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut half: Option<u8> = None;
    for &b in data {
        match b {
            b'>' => break,
            b if b.is_ascii_hexdigit() => {
                let value = match b {
                    b'0'..=b'9' => b - b'0',
                    b'a'..=b'f' => b - b'a' + 10,
                    _ => b - b'A' + 10,
                };
                match half.take() {
                    Some(hi) => out.push(hi * 16 + value),
                    None => half = Some(value),
                }
            }
            _ => {}
        }
    }
    if let Some(hi) = half {
        out.push(hi * 16);
    }
    out
}

/// §7.4.3, base-85. `z` is four zero bytes, `~>` ends the data, and a partial
/// final group encodes fewer than four bytes.
fn ascii85(data: &[u8]) -> Result<Vec<u8>> {
    let data = data.strip_prefix(b"<~").unwrap_or(data);
    let mut out = Vec::with_capacity(data.len() * 4 / 5);
    let mut group = [0u8; 5];
    let mut filled = 0usize;

    let flush = |group: &[u8; 5], filled: usize, out: &mut Vec<u8>| {
        if filled == 0 {
            return;
        }
        let mut value: u32 = 0;
        for (i, digit) in group.iter().enumerate() {
            // A short final group is padded with the highest digit, `u`.
            let digit = if i < filled { *digit } else { 84 };
            value = value.wrapping_mul(85).wrapping_add(u32::from(digit));
        }
        out.extend_from_slice(&value.to_be_bytes()[..filled - 1]);
    };

    for b in data.iter().copied() {
        match b {
            b'~' => break,
            b'z' if filled == 0 => out.extend_from_slice(&[0, 0, 0, 0]),
            b'z' => return Err(Error::Format("ASCII85Decode: `z` inside a group".into())),
            b'!'..=b'u' => {
                group[filled] = b - b'!';
                filled += 1;
                if filled == 5 {
                    flush(&group, 5, &mut out);
                    filled = 0;
                }
            }
            b if crate::object::is_whitespace(b) => {}
            other => {
                return Err(Error::Format(format!(
                    "ASCII85Decode: `{}` is not a base-85 digit",
                    other as char
                )));
            }
        }
    }
    if filled == 1 {
        return Err(Error::Format(
            "ASCII85Decode: a final group of one digit encodes no bytes".into(),
        ));
    }
    flush(&group, filled, &mut out);
    Ok(out)
}

/// §7.4.5. A length byte under 128 introduces that many literal bytes plus
/// one; over 128 it repeats the next byte `257 - length` times; 128 ends the
/// data.
fn run_length(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        let length = data[i];
        i += 1;
        match length {
            128 => break,
            0..=127 => {
                let count = usize::from(length) + 1;
                let end = (i + count).min(data.len());
                out.extend_from_slice(&data[i..end]);
                i = end;
            }
            _ => {
                let Some(&byte) = data.get(i) else { break };
                out.extend(std::iter::repeat_n(byte, 257 - usize::from(length)));
                i += 1;
            }
        }
    }
    out
}

/// §7.4.4.2's variable-code LZW, which is TIFF's rather than GIF's: codes grow
/// from nine bits, 256 clears the table and 257 ends the data.
///
/// `early` is `/EarlyChange`, and it is not cosmetic — it decides whether the
/// code width grows one entry before the table is full. A decoder on the wrong
/// side of it produces plausible-looking rubbish rather than an error.
fn lzw(data: &[u8], early: bool) -> Result<Vec<u8>> {
    const CLEAR: u16 = 256;
    const END: u16 = 257;

    let mut table: Vec<Vec<u8>> = Vec::new();
    let reset = |table: &mut Vec<Vec<u8>>| {
        table.clear();
        table.extend((0u16..=255).map(|b| vec![b as u8]));
        table.push(Vec::new()); // 256, clear
        table.push(Vec::new()); // 257, end
    };
    reset(&mut table);

    let mut out = Vec::new();
    let mut width = 9u32;
    let mut previous: Option<u16> = None;
    let mut bits: u32 = 0;
    let mut held: u32 = 0;

    for &byte in data {
        bits = (bits << 8) | u32::from(byte);
        held += 8;
        while held >= width {
            let code = ((bits >> (held - width)) & ((1 << width) - 1)) as u16;
            held -= width;

            match code {
                CLEAR => {
                    reset(&mut table);
                    width = 9;
                    previous = None;
                }
                END => return Ok(out),
                _ => {
                    let entry = match table.get(usize::from(code)) {
                        Some(entry) => entry.clone(),
                        None => {
                            // The one legal forward reference: the code being
                            // defined by this very step.
                            let Some(previous) = previous.and_then(|p| table.get(usize::from(p)))
                            else {
                                return Err(Error::Format(format!(
                                    "LZWDecode: code {code} is not in the table"
                                )));
                            };
                            let mut entry = previous.clone();
                            let Some(&first) = previous.first() else {
                                return Err(Error::Format(
                                    "LZWDecode: a code expanding to nothing".into(),
                                ));
                            };
                            entry.push(first);
                            entry
                        }
                    };
                    out.extend_from_slice(&entry);
                    if let Some(previous) = previous
                        && let Some(prefix) = table.get(usize::from(previous))
                    {
                        let mut new = prefix.clone();
                        new.push(entry[0]);
                        table.push(new);
                    }
                    previous = Some(code);

                    let limit = table.len() as u32 + u32::from(early);
                    width = match limit {
                        ..512 => 9,
                        512..1024 => 10,
                        1024..2048 => 11,
                        _ => 12,
                    };
                    if table.len() >= 4096 {
                        reset(&mut table);
                        width = 9;
                        previous = None;
                    }
                }
            }
        }
    }
    // Data that ran out before its end-of-data code: keep what decoded, for the
    // reason the module note gives.
    Ok(out)
}
