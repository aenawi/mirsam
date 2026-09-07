#!/usr/bin/env python3
"""Generate the PDF fixture corpus the object layer is proved against.

Writes into crates/mirsam-pdf/tests/pdfs/. One file per *generator family*,
because the things PLAN §6 hunts are generator signatures rather than author
mistakes: how a producer assembles a file decides what a reader has to be able
to do before it can see a single character.

  classic.pdf
      A cross-reference *table*, uncompressed content, no object streams —
      PDF 1.4 as anything predating 2003 writes it, and as a great many
      report generators still do. The baseline: if this does not read,
      nothing else will.

  compressed.pdf
      A cross-reference *stream* and an object stream, PDF 1.7. This is what
      LibreOffice, modern Word and Chrome's print-to-PDF emit, and every
      object but the stream containers themselves is inside a FlateDecode
      stream. A reader with no inflate sees an empty document.

  hybrid.pdf
      A classic table *and* an /XRefStm beside it (ISO 32000-1 §7.5.8.4) —
      Acrobat's compatibility output, where a 1.4 reader is shown a subset of
      what a 1.5 reader is shown. The page's real content object is reachable
      only through the stream, so a reader that took the classic table as the
      whole truth draws a blank page and reports no defects.

  incremental.pdf
      classic.pdf plus an appended update chained by /Prev, replacing the
      page's content. Every form filler, signer and annotation tool writes
      this shape. Newest-wins is the whole test: read the first definition
      and the document says the wrong thing.

  damaged.pdf
      Correct objects, a startxref pointing into the middle of nothing. What
      a truncated download, a naive concatenation or a hand edit leaves
      behind. Every viewer opens it by scanning for `obj`, so mirsam must
      too: a document reported unreadable while a reader opens it is the
      wrong half of standing rule 4.

  encrypted.pdf
      /Encrypt with the AES-256 crypt filter, V5 R6. Must be refused *by
      name*. The streams inside are not real ciphertext and this file is not
      a cryptographic fixture — nothing here decrypts, and what is being
      proved is that the refusal reads the /Encrypt dictionary, which
      §7.6.1 keeps in the clear precisely so a reader can say what it is up
      against.

  ascii-filters.pdf
      ASCIIHexDecode, ASCII85Decode, RunLengthDecode and LZWDecode, one per
      stream, plus an ASCII85+Flate chain. The distiller family: dvips and
      pdfTeX emitted ASCII-safe streams for decades so that a PDF survived a
      mail gateway, and the files are still in circulation.

  image-only.pdf
      A page whose entire content is a DCTDecode image. The scanner family,
      and the one this crate must be *unable* to read: the text is not in the
      file. It has to come back named in `unread` rather than as a page with
      nothing wrong with it.

Every one of them is written byte by byte here, with no third-party module,
for the reason the OOXML generators avoid python-pptx: a fixture built by the
library under test proves the library agrees with itself.

Regenerate with `make pdfs`.
"""

from __future__ import annotations

import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "crates" / "mirsam-pdf" / "tests" / "pdfs"

# The Arabic every fixture carries, so §6.2 has something to grow into and so a
# reader that silently drops non-ASCII fails here rather than on a user's file.
ARABIC = "التقرير السنوي"


def utf16be(text: str) -> bytes:
    """A PDF text string in UTF-16BE with the byte-order mark §7.9.2.2 wants."""
    return b"\xfe\xff" + text.encode("utf-16-be")


def hex_string(data: bytes) -> bytes:
    return b"<" + data.hex().upper().encode("ascii") + b">"


def tounicode(codes: dict[int, str]) -> bytes:
    """A ToUnicode CMap mapping single-byte codes to the characters they draw.

    This is how a generator says what its glyph codes mean, and the only thing
    standing between an extractor and a page of numbers.
    """
    pairs = b"".join(
        b"<%02X> <%s>\n" % (code, char.encode("utf-16-be").hex().upper().encode("ascii"))
        for code, char in sorted(codes.items())
    )
    return (
        b"/CIDInit /ProcSet findresource begin\n"
        b"12 dict begin\nbegincmap\n"
        b"/CMapName /Mirsam-Fixture def\n/CMapType 2 def\n"
        b"1 begincodespacerange\n<00> <FF>\nendcodespacerange\n"
        + b"%d beginbfchar\n" % len(codes)
        + pairs
        + b"endbfchar\n"
        b"endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n"
    )


def arabic_codes(text: str, first: int = 0x41) -> tuple[bytes, dict[int, str]]:
    """One byte per character, plus the map back. The shape a generator that
    subsets a font produces, and the shape §6.3 hunts a reversal in."""
    codes: dict[int, str] = {}
    out = bytearray()
    for index, char in enumerate(text):
        code = first + index
        codes[code] = char
        out.append(code)
    return bytes(out), codes


class Builder:
    """Objects in, PDF out. Holds the offsets a cross-reference table needs."""

    def __init__(self, version: str = "1.4") -> None:
        self.version = version
        self.objects: dict[int, bytes] = {}
        self.next = 1

    def add(self, body: bytes, number: int | None = None) -> int:
        if number is None:
            number = self.next
        self.next = max(self.next, number + 1)
        self.objects[number] = body
        return number

    def stream(self, dict_entries: bytes, data: bytes, number: int | None = None) -> int:
        body = b"<< " + dict_entries + b" /Length %d >>\nstream\n" % len(data) + data + b"\nendstream"
        return self.add(body, number)

    def serialise(self, root: int, info: int | None = None, extra_trailer: bytes = b"") -> bytes:
        out = bytearray(b"%%PDF-%s\n%%\xe2\xe3\xcf\xd3\n" % self.version.encode("ascii"))
        offsets: dict[int, int] = {}
        for number in sorted(self.objects):
            offsets[number] = len(out)
            out += b"%d 0 obj\n" % number + self.objects[number] + b"\nendobj\n"

        size = max(self.objects) + 1
        startxref = len(out)
        out += b"xref\n0 %d\n" % size
        out += b"0000000000 65535 f \n"
        for number in range(1, size):
            if number in offsets:
                out += b"%010d 00000 n \n" % offsets[number]
            else:
                out += b"0000000000 65535 f \n"
        trailer = b"<< /Size %d /Root %d 0 R" % (size, root)
        if info is not None:
            trailer += b" /Info %d 0 R" % info
        trailer += extra_trailer + b" >>"
        out += b"trailer\n" + trailer + b"\nstartxref\n%d\n%%%%EOF\n" % startxref
        return bytes(out)


def page_document(builder: Builder, content: bytes, *, font_extra: bytes = b"") -> tuple[int, int]:
    """The four objects every one-page fixture shares. Returns (catalog, content)."""
    catalog = builder.add(b"<< /Type /Catalog /Pages 2 0 R >>", 1)
    builder.add(b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 595 842] >>", 2)
    builder.add(
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R"
        b" /Resources << /Font << /F1 5 0 R >> >> >>",
        3,
    )
    stream = builder.stream(b"", content, 4)
    builder.add(
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica" + font_extra + b" >>",
        5,
    )
    return catalog, stream


def content_for(text: str, *, first: int = 0x41) -> tuple[bytes, dict[int, str]]:
    codes, mapping = arabic_codes(text, first)
    stream = (
        b"BT\n/F1 18 Tf\n72 760 Td\n(Annual report) Tj\n"
        b"0 -28 Td\n" + hex_string(codes) + b" Tj\nET\n"
    )
    return stream, mapping


def write(name: str, data: bytes) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / name).write_bytes(data)
    print(f"  {name:22} {len(data):7} bytes")


# --------------------------------------------------------------------------
# classic.pdf
# --------------------------------------------------------------------------
def classic() -> bytes:
    builder = Builder("1.4")
    content, mapping = content_for(ARABIC)
    catalog, _ = page_document(builder, content, font_extra=b" /ToUnicode 6 0 R")
    builder.stream(b"", tounicode(mapping), 6)
    info = builder.add(
        b"<< /Title " + hex_string(utf16be(ARABIC)) + b" /Producer (mirsam fixtures) >>", 7
    )
    return builder.serialise(catalog, info)


# --------------------------------------------------------------------------
# compressed.pdf — cross-reference stream and object stream
# --------------------------------------------------------------------------
def compressed() -> bytes:
    content, mapping = content_for(ARABIC)
    content_z = zlib.compress(content, 9)
    cmap_z = zlib.compress(tounicode(mapping), 9)

    # 1..3 and 7 go inside the object stream; 4, 5 and 6 are streams, which
    # §7.5.7 forbids an object stream from holding.
    inside = {
        1: b"<< /Type /Catalog /Pages 2 0 R >>",
        2: b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 595 842] >>",
        3: (
            b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R"
            b" /Resources << /Font << /F1 5 0 R >> >> >>"
        ),
        5: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>",
        7: b"<< /Title " + hex_string(utf16be(ARABIC)) + b" /Producer (mirsam fixtures) >>",
    }
    pairs = bytearray()
    bodies = bytearray()
    for number, body in sorted(inside.items()):
        pairs += b"%d %d " % (number, len(bodies))
        bodies += body + b" "
    objstm_plain = bytes(pairs) + bytes(bodies)
    first = len(pairs)
    objstm_z = zlib.compress(objstm_plain, 9)

    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets: dict[int, int] = {}

    def emit(number: int, body: bytes) -> None:
        offsets[number] = len(out)
        out.extend(b"%d 0 obj\n" % number + body + b"\nendobj\n")

    emit(
        4,
        b"<< /Filter /FlateDecode /Length %d >>\nstream\n" % len(content_z)
        + content_z
        + b"\nendstream",
    )
    emit(
        6,
        b"<< /Filter /FlateDecode /Length %d >>\nstream\n" % len(cmap_z) + cmap_z + b"\nendstream",
    )
    emit(
        8,
        b"<< /Type /ObjStm /N %d /First %d /Filter /FlateDecode /Length %d >>\nstream\n"
        % (len(inside), first, len(objstm_z))
        + objstm_z
        + b"\nendstream",
    )

    # 9 is the cross-reference stream itself: three fields, 1/4/2 bytes wide.
    size = 10
    rows = bytearray()
    startxref = len(out)
    for number in range(size):
        if number == 0:
            rows += bytes([0]) + (0).to_bytes(4, "big") + (0xFFFF).to_bytes(2, "big")
        elif number in offsets:
            rows += bytes([1]) + offsets[number].to_bytes(4, "big") + (0).to_bytes(2, "big")
        elif number == 9:
            rows += bytes([1]) + startxref.to_bytes(4, "big") + (0).to_bytes(2, "big")
        elif number in inside:
            index = sorted(inside).index(number)
            rows += bytes([2]) + (8).to_bytes(4, "big") + index.to_bytes(2, "big")
        else:
            rows += bytes([0]) + (0).to_bytes(4, "big") + (0xFFFF).to_bytes(2, "big")
    rows_z = zlib.compress(bytes(rows), 9)
    out.extend(
        b"9 0 obj\n<< /Type /XRef /Size %d /W [1 4 2] /Root 1 0 R /Info 7 0 R"
        b" /Filter /FlateDecode /Length %d >>\nstream\n" % (size, len(rows_z))
        + rows_z
        + b"\nendstream\nendobj\n"
    )
    out.extend(b"startxref\n%d\n%%%%EOF\n" % startxref)
    return bytes(out)


# --------------------------------------------------------------------------
# hybrid.pdf — a classic table with /XRefStm beside it
# --------------------------------------------------------------------------
def hybrid() -> bytes:
    content, mapping = content_for(ARABIC)
    cmap = tounicode(mapping)

    out = bytearray(b"%PDF-1.5\n%\xe2\xe3\xcf\xd3\n")
    offsets: dict[int, int] = {}

    def emit(number: int, body: bytes) -> None:
        offsets[number] = len(out)
        out.extend(b"%d 0 obj\n" % number + body + b"\nendobj\n")

    emit(1, b"<< /Type /Catalog /Pages 2 0 R >>")
    emit(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 595 842] >>")
    emit(
        3,
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R"
        b" /Resources << /Font << /F1 5 0 R >> >> >>",
    )
    emit(4, b"<< /Length %d >>\nstream\n" % len(content) + content + b"\nendstream")
    emit(5, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>")
    # 6 is the object the classic table deliberately does not name: only the
    # /XRefStm knows where it is, which is the whole point of a hybrid file.
    emit(6, b"<< /Length %d >>\nstream\n" % len(cmap) + cmap + b"\nendstream")

    size = 8
    rows = bytearray()
    xrefstm_at = len(out)
    for number in range(size):
        if number in offsets:
            rows += bytes([1]) + offsets[number].to_bytes(4, "big") + (0).to_bytes(2, "big")
        elif number == 7:
            rows += bytes([1]) + xrefstm_at.to_bytes(4, "big") + (0).to_bytes(2, "big")
        else:
            rows += bytes([0]) + (0).to_bytes(4, "big") + (0xFFFF).to_bytes(2, "big")
    rows_z = zlib.compress(bytes(rows), 9)
    out.extend(
        b"7 0 obj\n<< /Type /XRef /Size %d /W [1 4 2] /Root 1 0 R"
        b" /Filter /FlateDecode /Length %d >>\nstream\n" % (size, len(rows_z))
        + rows_z
        + b"\nendstream\nendobj\n"
    )

    startxref = len(out)
    out.extend(b"xref\n0 6\n0000000000 65535 f \n")
    for number in range(1, 6):
        out.extend(b"%010d 00000 n \n" % offsets[number])
    out.extend(
        b"trailer\n<< /Size %d /Root 1 0 R /XRefStm %d >>\nstartxref\n%d\n%%%%EOF\n"
        % (size, xrefstm_at, startxref)
    )
    return bytes(out)


# --------------------------------------------------------------------------
# incremental.pdf — classic.pdf, then an update that supersedes the content
# --------------------------------------------------------------------------
def incremental() -> bytes:
    base = classic()
    first_startxref = int(base.rsplit(b"startxref\n", 1)[1].split(b"\n", 1)[0])

    revised, _ = content_for("التقرير السنوي المعدل")
    out = bytearray(base)
    offset = len(out)
    out.extend(
        b"4 0 obj\n<< /Length %d >>\nstream\n" % len(revised) + revised + b"\nendstream\nendobj\n"
    )
    startxref = len(out)
    out.extend(
        b"xref\n0 1\n0000000000 65535 f \n4 1\n%010d 00000 n \n" % offset
        + b"trailer\n<< /Size 8 /Root 1 0 R /Info 7 0 R /Prev %d >>\nstartxref\n%d\n%%%%EOF\n"
        % (first_startxref, startxref)
    )
    return bytes(out)


# --------------------------------------------------------------------------
# damaged.pdf — good objects, a startxref pointing nowhere useful
# --------------------------------------------------------------------------
def damaged() -> bytes:
    base = classic()
    head, _ = base.rsplit(b"startxref\n", 1)
    # Past the end of the file, which is what a truncated transfer leaves and
    # what a reader must survive by scanning the body instead.
    return head + b"startxref\n%d\n%%%%EOF\n" % (len(base) + 4096)


# --------------------------------------------------------------------------
# encrypted.pdf — refused by name, never decrypted
# --------------------------------------------------------------------------
def encrypted() -> bytes:
    builder = Builder("1.7")
    content, mapping = content_for(ARABIC)
    catalog, _ = page_document(builder, content, font_extra=b" /ToUnicode 6 0 R")
    builder.stream(b"", tounicode(mapping), 6)
    encrypt = builder.add(
        b"<< /Filter /Standard /V 5 /R 6 /Length 256"
        b" /CF << /StdCF << /CFM /AESV3 /AuthEvent /DocOpen /Length 32 >> >>"
        b" /StmF /StdCF /StrF /StdCF"
        b" /O <" + b"00" * 48 + b"> /U <" + b"00" * 48 + b">"
        b" /OE <" + b"00" * 32 + b"> /UE <" + b"00" * 32 + b">"
        b" /Perms <" + b"00" * 16 + b"> /P -1340 >>",
        7,
    )
    return builder.serialise(catalog, extra_trailer=b" /Encrypt %d 0 R" % encrypt)


# --------------------------------------------------------------------------
# ascii-filters.pdf — the distiller family
# --------------------------------------------------------------------------
def ascii_hex(data: bytes) -> bytes:
    return data.hex().upper().encode("ascii") + b">"


def ascii85(data: bytes) -> bytes:
    out = bytearray()
    for start in range(0, len(data), 4):
        group = data[start : start + 4]
        pad = 4 - len(group)
        value = int.from_bytes(group + b"\0" * pad, "big")
        if value == 0 and pad == 0:
            out += b"z"
            continue
        digits = bytearray()
        for _ in range(5):
            digits.insert(0, 33 + value % 85)
            value //= 85
        out += bytes(digits)[: 5 - pad]
    return bytes(out) + b"~>"


def run_length(data: bytes) -> bytes:
    """Literal runs only. A decoder that mishandles the length byte produces
    plausible rubbish, which is what this is here to catch."""
    out = bytearray()
    for start in range(0, len(data), 128):
        chunk = data[start : start + 128]
        out.append(len(chunk) - 1)
        out += chunk
    return bytes(out) + b"\x80"


def lzw(data: bytes) -> bytes:
    """Variable-code LZW with early change, as §7.4.4.2 defines it."""
    table = {bytes([b]): b for b in range(256)}
    nxt = 258
    width = 9
    bits = 0
    held = 0
    out = bytearray()

    def push(code: int) -> None:
        nonlocal bits, held
        bits = (bits << width) | code
        held += width
        while held >= 8:
            out.append((bits >> (held - 8)) & 0xFF)
            held -= 8

    push(256)
    current = b""
    for byte in data:
        candidate = current + bytes([byte])
        if candidate in table:
            current = candidate
            continue
        push(table[current])
        table[candidate] = nxt
        nxt += 1
        # Early change: the width grows one code before the table is full.
        if nxt + 1 > (1 << width) and width < 12:
            width += 1
        current = bytes([byte])
    if current:
        push(table[current])
    push(257)
    if held:
        out.append((bits << (8 - held)) & 0xFF)
    return bytes(out)


def ascii_filters() -> bytes:
    builder = Builder("1.4")
    content, mapping = content_for(ARABIC)
    cmap = tounicode(mapping)

    catalog = builder.add(b"<< /Type /Catalog /Pages 2 0 R >>", 1)
    builder.add(b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 595 842] >>", 2)
    builder.add(
        b"<< /Type /Page /Parent 2 0 R /Contents [4 0 R 8 0 R 9 0 R]"
        b" /Resources << /Font << /F1 5 0 R >> >> >>",
        3,
    )
    # The page's own content, through ASCII85 and Flate in that order — the
    # chain a distiller writes, and the one a reader must unwind in reverse.
    chained = ascii85(zlib.compress(content, 9))
    builder.stream(b"/Filter [/ASCII85Decode /FlateDecode]", chained, 4)
    builder.add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>", 5)
    builder.stream(b"/Filter /ASCIIHexDecode", ascii_hex(cmap), 6)
    builder.add(b"<< /Producer (mirsam fixtures) >>", 7)
    builder.stream(b"/Filter /RunLengthDecode", run_length(b"% run length\n"), 8)
    builder.stream(b"/Filter /LZWDecode", lzw(b"% lzw\n"), 9)
    return builder.serialise(catalog, 7)


# --------------------------------------------------------------------------
# image-only.pdf — the scanner family, which has no text to find
# --------------------------------------------------------------------------
# The smallest thing a JPEG decoder would accept, and this one never runs: what
# matters is that the filter is named and the bytes are left alone.
JPEG = bytes.fromhex(
    "ffd8ffe000104a46494600010100000100010000ffdb004300"
    + "08" * 64
    + "ffc0000b080001000101011100ffc40014000100000000000000000000000000000000"
    "03ffda0008010100003f00d2cf20ffd9"
)


def image_only() -> bytes:
    builder = Builder("1.4")
    content = b"q\n595 0 0 842 0 0 cm\n/Im1 Do\nQ\n"
    catalog = builder.add(b"<< /Type /Catalog /Pages 2 0 R >>", 1)
    builder.add(b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 595 842] >>", 2)
    builder.add(
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R"
        b" /Resources << /XObject << /Im1 5 0 R >> >> >>",
        3,
    )
    builder.stream(b"", content, 4)
    builder.stream(
        b"/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray"
        b" /BitsPerComponent 8 /Filter /DCTDecode",
        JPEG,
        5,
    )
    return builder.serialise(catalog)


def main() -> None:
    print(f"writing PDF fixtures into {OUT}")
    write("classic.pdf", classic())
    write("compressed.pdf", compressed())
    write("hybrid.pdf", hybrid())
    write("incremental.pdf", incremental())
    write("damaged.pdf", damaged())
    write("encrypted.pdf", encrypted())
    write("ascii-filters.pdf", ascii_filters())
    write("image-only.pdf", image_only())


if __name__ == "__main__":
    main()
