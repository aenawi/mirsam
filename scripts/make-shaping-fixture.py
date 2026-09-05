#!/usr/bin/env python3
"""Generate the three hand-built test fonts the shaping tests run against.

Writes into crates/mirsam-core/tests/fonts/:

  joining.ttf
      An Arabic font that shapes: it maps U+0621..U+064A, and its GSUB
      carries the three form features a shaper needs — `init`, `medi`,
      `fina` — each a single substitution moving a standalone glyph to the
      contextual one. Text shaped through it must come back joined.

  nonjoining.ttf
      The same cmap, the same glyph order, no GSUB at all. This is the
      defect M4 exists to catch: a font that answers for every Arabic
      codepoint, so nothing looks missing, and renders every letter in its
      standalone form because it has no shaping tables. Text shaped through
      it must come back unjoined.

  partial.ttf
      `init` and `medi` in full, and a `fina` that covers U+0633..U+064A
      only — so the final forms of alef, dal, reh, waw and their kin come
      back as the standalone glyph. This is not a defect and not an
      invention: Arial does exactly this. A letter that only ever takes a
      join from its right needs no glyph of its own for it, because the
      connecting stroke belongs to the letter before it. Shaping `مرحبا`
      through macOS's Arial leaves the reh on its standalone glyph and the
      word renders perfectly. The fixture exists so that a rule which
      concluded a defect from one such letter fails a test here rather than
      on a user's deck.

Three fonts, one difference each, so a test that separates them is proving
the detection and not the shaper.

The glyph order is the whole trick, and the tests depend on it:

    0            .notdef
    1            space
    2   .. 43    standalone forms of U+0621..U+064A, in codepoint order
    44  .. 85    initial forms, at standalone + 42
    86  .. 127   medial forms,  at standalone + 84
    128 .. 169   final forms,   at standalone + 126

so each form feature is one SingleSubstFormat1 lookup with a constant delta
over one coverage range, and a test can name the glyph it expects. Narrowing
a lookup's coverage range is then all it takes to build a font that shapes
some letters and not others.

The glyphs have no outlines. Shaping never reads them, and an empty `glyf`
keeps the fixture at a size worth committing (~2 KB).

Written with `struct` and nothing else ON PURPOSE, for the reason
`make-torture-fixture.py` gives: a fixture built with the library under test
only proves the library agrees with itself. Nothing here has seen
`ttf-parser` or `rustybuzz`.

Deterministic: fixed dates, no clock, no randomness, so re-running
reproduces the committed bytes exactly.

Usage:  python3 scripts/make-shaping-fixture.py [output-directory]
"""

from __future__ import annotations

import os
import struct
import sys

OUT_DIR = sys.argv[1] if len(sys.argv) > 1 else "crates/mirsam-core/tests/fonts"

# ------------------------------------------------------------------ the glyphs

FIRST_CP = 0x0621  # ARABIC LETTER HAMZA
LAST_CP = 0x064A  # ARABIC LETTER YEH
LETTERS = LAST_CP - FIRST_CP + 1  # 42

GID_SPACE = 1
GID_ISOL = 2
GID_INIT = GID_ISOL + LETTERS
GID_MEDI = GID_INIT + LETTERS
GID_FINA = GID_MEDI + LETTERS
NUM_GLYPHS = GID_FINA + LETTERS  # 170

UNITS_PER_EM = 1000
ADVANCE = 600

# 1 January 2026, 00:00:00 UTC, in seconds since the 1904 Mac epoch. A
# generator that read the clock would produce a diff on every run.
EPOCH_1904_TO_1970 = 2082844800
CREATED = EPOCH_1904_TO_1970 + 1767225600


def pad4(data: bytes) -> bytes:
    return data + b"\0" * (-len(data) % 4)


def checksum(data: bytes) -> int:
    data = pad4(data)
    total = 0
    for (word,) in struct.iter_unpack(">I", data):
        total = (total + word) & 0xFFFFFFFF
    return total


# ------------------------------------------------------------------ the tables


def head() -> bytes:
    return struct.pack(
        ">IIIIHHqqhhhhHHhhh",
        0x00010000,  # version
        0x00010000,  # fontRevision
        0,  # checkSumAdjustment, patched once the file is assembled
        0x5F0F3CF5,  # magicNumber
        0x0003,  # flags: baseline at y=0, lsb at x=0
        UNITS_PER_EM,
        CREATED,
        CREATED,
        0,
        0,
        ADVANCE,
        UNITS_PER_EM,  # bounding box
        0,  # macStyle
        8,  # lowestRecPPEM
        2,  # fontDirectionHint
        0,  # indexToLocFormat: short
        0,  # glyphDataFormat
    )


def hhea() -> bytes:
    return struct.pack(
        ">IhhhHhhhhhhhhhhhH",
        0x00010000,
        800,  # ascender
        -200,  # descender
        0,  # lineGap
        ADVANCE,  # advanceWidthMax
        0,  # minLeftSideBearing
        0,  # minRightSideBearing
        ADVANCE,  # xMaxExtent
        1,  # caretSlopeRise
        0,  # caretSlopeRun
        0,  # caretOffset
        0,
        0,
        0,
        0,  # reserved
        0,  # metricDataFormat
        NUM_GLYPHS,  # numberOfHMetrics
    )


def maxp() -> bytes:
    return struct.pack(">IH", 0x00010000, NUM_GLYPHS) + b"\0" * 26


def hmtx() -> bytes:
    return struct.pack(">Hh", ADVANCE, 0) * NUM_GLYPHS


def loca() -> bytes:
    # Short format, every offset zero: every glyph is empty.
    return b"\0\0" * (NUM_GLYPHS + 1)


def glyf() -> bytes:
    # No outlines. Shaping reads the tables above this one, never this.
    return b"\0\0\0\0"


def cmap() -> bytes:
    # Three segments, sorted by end code: the space, the letters, and the
    # 0xFFFF terminator every format 4 subtable is required to end with.
    segments = [
        (0x0020, 0x0020, (GID_SPACE - 0x0020) & 0xFFFF),
        (FIRST_CP, LAST_CP, (GID_ISOL - FIRST_CP) & 0xFFFF),
        (0xFFFF, 0xFFFF, 1),
    ]
    seg_count = len(segments)
    search_range = 2 * (2 ** (seg_count.bit_length() - 1))

    body = struct.pack(">HHH", seg_count * 2, search_range, seg_count.bit_length() - 1)
    body += struct.pack(">H", seg_count * 2 - search_range)  # rangeShift
    body += b"".join(struct.pack(">H", end) for _, end, _ in segments)
    body += struct.pack(">H", 0)  # reservedPad
    body += b"".join(struct.pack(">H", start) for start, _, _ in segments)
    body += b"".join(struct.pack(">H", delta) for _, _, delta in segments)
    body += struct.pack(">H", 0) * seg_count  # idRangeOffset: delta only

    subtable = struct.pack(">HHH", 4, len(body) + 6, 0) + body
    header = struct.pack(">HH", 0, 1) + struct.pack(">HHI", 3, 1, 12)
    return header + subtable


def name() -> bytes:
    records = [
        (1, "Mirsam Shaping Test"),
        (2, "Regular"),
        (4, "Mirsam Shaping Test"),
        (6, "MirsamShapingTest"),
    ]
    strings = b""
    entries = b""
    for name_id, value in records:
        encoded = value.encode("utf-16-be")
        entries += struct.pack(">HHHHHH", 3, 1, 0x0409, name_id, len(encoded), len(strings))
        strings += encoded
    header = struct.pack(">HHH", 0, len(records), 6 + 12 * len(records))
    return header + entries + strings


def post() -> bytes:
    return struct.pack(">IihhIIIII", 0x00030000, 0, -100, 50, 0, 0, 0, 0, 0)


def os2() -> bytes:
    return (
        struct.pack(
            ">HhHHH",
            4,  # version
            ADVANCE,  # xAvgCharWidth
            400,  # usWeightClass
            5,  # usWidthClass
            0,  # fsType
        )
        + struct.pack(">hhhh", 650, 600, 0, 75)  # subscript
        + struct.pack(">hhhh", 650, 600, 0, 350)  # superscript
        + struct.pack(">hh", 50, 250)  # strikeout
        + struct.pack(">h", 0)  # sFamilyClass
        + b"\0" * 10  # panose
        + struct.pack(">IIII", 1 << 13, 0, 0, 0)  # unicode ranges: Arabic
        + b"MRSM"  # achVendID
        + struct.pack(">HHH", 0x0040, 0x0020, LAST_CP)  # regular, first, last
        + struct.pack(">hhh", 800, -200, 0)  # typo ascender/descender/gap
        + struct.pack(">HH", 800, 200)  # win ascent/descent
        + struct.pack(">II", 0, 0)  # code page ranges
        + struct.pack(">hh", 500, 700)  # x-height, cap height
        + struct.pack(">HHH", 0, 0x0020, 1)  # default, break, max context
    )


def gsub(fina_from: int = FIRST_CP) -> bytes:
    """One script, three form features, three single-substitution lookups.

    `fina_from` is the first codepoint the `fina` lookup covers. Left at the
    default it covers every letter; raised, the letters below it keep their
    standalone glyph in final position, which is what `partial.ttf` is for.
    """

    def coverage(first_cp: int) -> bytes:
        first = GID_ISOL + (first_cp - FIRST_CP)
        return struct.pack(">HH", 2, 1) + struct.pack(
            ">HHH", first, GID_ISOL + LETTERS - 1, 0
        )

    # Lookups, in the order the features below index them.
    lookups_wanted = [
        (GID_FINA - GID_ISOL, coverage(fina_from)),  # fina
        (GID_INIT - GID_ISOL, coverage(FIRST_CP)),  # init
        (GID_MEDI - GID_ISOL, coverage(FIRST_CP)),  # medi
    ]

    lookups = b""
    lookup_offsets = []
    base = 2 + 2 * len(lookups_wanted)  # lookupCount + offsets
    for delta, cov in lookups_wanted:
        # Lookup header, then its one SingleSubstFormat1 subtable, then the
        # coverage that subtable points at.
        subtable = struct.pack(">HHh", 1, 6, delta) + cov
        lookup = struct.pack(">HHHH", 1, 0, 1, 8) + subtable
        lookup_offsets.append(base + len(lookups))
        lookups += lookup
    lookup_list = (
        struct.pack(">H", len(lookups_wanted))
        + b"".join(struct.pack(">H", off) for off in lookup_offsets)
        + lookups
    )

    # Feature records must be sorted by tag: fina < init < medi.
    tags = [b"fina", b"init", b"medi"]
    features = b""
    feature_offsets = []
    base = 2 + 6 * len(tags)
    for index in range(len(tags)):
        feature_offsets.append(base + len(features))
        features += struct.pack(">HHH", 0, 1, index)  # params, count, lookup
    feature_list = (
        struct.pack(">H", len(tags))
        + b"".join(
            tag + struct.pack(">H", off) for tag, off in zip(tags, feature_offsets)
        )
        + features
    )

    lang_sys = struct.pack(">HHH", 0, 0xFFFF, len(tags)) + b"".join(
        struct.pack(">H", i) for i in range(len(tags))
    )
    script = struct.pack(">HH", 4, 0) + lang_sys  # defaultLangSys, no others
    script_list = struct.pack(">H", 1) + b"arab" + struct.pack(">H", 8) + script

    header_len = 10
    script_off = header_len
    feature_off = script_off + len(script_list)
    lookup_off = feature_off + len(feature_list)
    header = struct.pack(">HHHHH", 1, 0, script_off, feature_off, lookup_off)
    return header + script_list + feature_list + lookup_list


# ------------------------------------------------------------------ the package


def assemble(tables: dict[str, bytes]) -> bytes:
    """Table directory, tables, then the head checksum the spec asks for."""
    tags = sorted(tables)
    num = len(tags)
    search_range = 16 * (2 ** (num.bit_length() - 1))
    directory = struct.pack(
        ">IHHHH",
        0x00010000,
        num,
        search_range,
        num.bit_length() - 1,
        num * 16 - search_range,
    )

    offset = 12 + 16 * num
    records = b""
    body = b""
    for tag in tags:
        data = tables[tag]
        records += struct.pack(">4sIII", tag.encode("ascii"), checksum(data), offset, len(data))
        body += pad4(data)
        offset += len(pad4(data))

    font = directory + records + body

    # checkSumAdjustment: 0xB1B0AFBA less the checksum of the whole file with
    # that field still zero. Its own offset is inside the head table.
    head_offset = 12 + 16 * num + sum(
        len(pad4(tables[t])) for t in tags[: tags.index("head")]
    )
    adjustment = (0xB1B0AFBA - checksum(font)) & 0xFFFFFFFF
    at = head_offset + 8
    return font[:at] + struct.pack(">I", adjustment) + font[at + 4 :]


def build(shaping: bytes | None) -> bytes:
    tables = {
        "OS/2": os2(),
        "cmap": cmap(),
        "glyf": glyf(),
        "head": head(),
        "hhea": hhea(),
        "hmtx": hmtx(),
        "loca": loca(),
        "maxp": maxp(),
        "name": name(),
        "post": post(),
    }
    if shaping is not None:
        tables["GSUB"] = shaping
    return assemble(tables)


# U+0633 ARABIC LETTER SEEN: the first letter `partial.ttf` gives a final
# form to, so everything below it — alef, teh marbuta, dal, thal, reh, zain —
# keeps its standalone glyph there. Those are the right-joining letters, the
# ones a real font can leave alone for the reason `partial.ttf` documents.
PARTIAL_FINA_FROM = 0x0633

FONTS = {
    "joining.ttf": gsub(),
    "nonjoining.ttf": None,
    "partial.ttf": gsub(fina_from=PARTIAL_FINA_FROM),
}


def main() -> None:
    os.makedirs(OUT_DIR, exist_ok=True)
    for filename, shaping in FONTS.items():
        path = os.path.join(OUT_DIR, filename)
        data = build(shaping)
        with open(path, "wb") as handle:
            handle.write(data)
        print(f"wrote {path} ({len(data)} bytes, {NUM_GLYPHS} glyphs)")


if __name__ == "__main__":
    main()
