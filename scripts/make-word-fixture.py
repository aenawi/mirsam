#!/usr/bin/env python3
"""Generate the hand-built Word corpus documents.

Writes two documents:

  tests/fixtures/quarterly-review.docx
      One Word document carrying every defect the DOCX reader can find, next
      to text that is correct and text that is English. Where the corpus decks
      answer "what does mirsam do to a presentation", this answers the same
      question for the other format the tool reads — which is what makes the
      conformance suite's claim (PLAN §3.5) checkable against real bytes
      rather than only against packages a test built in memory.

  tests/fixtures/quarterly-review-correct.docx
      The same document authored correctly: explicit `w:bidi` on every Arabic
      paragraph, an Arabic language tag, a complex-script font wherever a
      Latin one is set, a real numbered list instead of a typed glyph, no
      controls, no presentation forms, and `w:bidiVisual` on the table.
      mirsam must leave it completely alone, which is what makes `audit`
      exit code 0 provable for `.docx` and not only for `.pptx`.

Both exercise Word's own chain, because a document that never inherits
anything would let a broken `StyleSheet` pass: `Normal` states the document's
right-to-left default, `EnglishBody` contradicts the Arabic under it — the
ADR 0007 case, reported as an absent value and naming the style that supplied
it — and `RtlTable` supplies a table's column order the table does not state.

Written with Python's zipfile ON PURPOSE, for the reason
`make-torture-fixture.py` gives: generating a fixture with the same `zip`
crate under test would only prove the writer agrees with itself.

Deterministic: fixed timestamps and a fixed compression level, so re-running
reproduces the committed bytes exactly.

Usage:  python3 scripts/make-word-fixture.py [output-directory]

Then regenerate the expected reports:  make golden
"""

from __future__ import annotations

import os
import posixpath
import sys
import urllib.parse
import zipfile

OUT_DIR = sys.argv[1] if len(sys.argv) > 1 else "tests/fixtures"

# One timestamp for every entry: the corpus is committed bytes, and a
# generator that used the clock would produce a diff on every run.
TIMESTAMP = (2026, 1, 1, 0, 0, 0)

PKG = "http://schemas.openxmlformats.org/package/2006"
OFFICE = "http://schemas.openxmlformats.org/officeDocument/2006"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"

PROLOG = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'

# ------------------------------------------------------------------- the text

# Mixed Arabic and Latin: the case where the base direction actually changes
# what a reader sees, because the digits and the `Q4` move.
MIXED = "ارتفع الأداء بنسبة 25% في Q4 2026."
ARABIC = "التقرير الفصلي للربع الرابع"
HEADING = "ملخص تنفيذي"
ENGLISH = "Revenue rose 25% in Q4 2026."

# Pasted out of a PDF: Arabic already shaped into presentation forms. It looks
# right in the page and is unsearchable, uncopyable and un-reshapeable.
PRESHAPED = "ﺍﻟﺘﻘﺮﻳﺮ ﺍﻟﻔﺼﻠﻲ"

# RLE … PDF around a mixed run: the workaround this tool exists to replace.
CONTROLS = "‫ارتفع الأداء بنسبة 25%‬"

TYPED_BULLET = "• نمو في قطاع التجزئة"

TABLE_CELLS = [
    ["المؤشر", "الربع الثالث", "الربع الرابع"],
    ["الإيرادات", "12.4", "15.5"],
]

HEADER = "شركة المثال — تقرير داخلي"


def escape(text: str) -> str:
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


# -------------------------------------------------------------- the vocabulary


def p_pr(*, style=None, num=False, bidi=None, jc=None, run=""):
    """A `w:pPr` with its children in the order CT_PPrBase declares them.

    The sequence is not decoration: `w:pPr` is an `xsd:sequence`, so a
    `w:bidi` written after a `w:jc` is a document Word offers to repair, and
    `scripts/validate-ooxml.py` fails it against `wml.xsd`.
    """
    body = ""
    if style is not None:
        body += f'<w:pStyle w:val="{style}"/>'
    if num:
        body += '<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>'
    if bidi is not None:
        body += f'<w:bidi w:val="{1 if bidi else 0}"/>'
    if jc is not None:
        body += f'<w:jc w:val="{jc}"/>'
    if run:
        body += f"<w:rPr>{run}</w:rPr>"
    return f"<w:pPr>{body}</w:pPr>" if body else ""


def r_pr(*, latin=None, complex_font=None, cstheme=None, lang=None, rtl=None):
    """A `w:rPr`, children again in CT_RPr's declared order."""
    body = ""
    fonts = ""
    if latin is not None:
        fonts += f' w:ascii="{latin}" w:hAnsi="{latin}"'
    if complex_font is not None:
        fonts += f' w:cs="{complex_font}"'
    if cstheme is not None:
        fonts += f' w:cstheme="{cstheme}"'
    if fonts:
        body += f"<w:rFonts{fonts}/>"
    if rtl is not None:
        body += f'<w:rtl w:val="{1 if rtl else 0}"/>'
    if lang is not None:
        body += f'<w:lang w:bidi="{lang}"/>'
    return body


def paragraph(text, *, style=None, num=False, bidi=None, jc=None, **run):
    """One `w:p` holding one run."""
    properties = r_pr(**run)
    return (
        f"<w:p>{p_pr(style=style, num=num, bidi=bidi, jc=jc, run=properties)}"
        f"<w:r>{f'<w:rPr>{properties}</w:rPr>' if properties else ''}"
        f'<w:t xml:space="preserve">{escape(text)}</w:t></w:r></w:p>'
    )


def table(rows, *, style=None, bidi_visual=None, cell_bidi=None, cell_jc=None,
          cell_run=None):
    """One `w:tbl`, cells stored in the order they are written above.

    `w:bidiVisual` says the cells are *displayed* right to left with the
    file's own order unchanged, so the same cell list produces both the
    correct table and the defective one.
    """
    properties = ""
    if style is not None:
        properties += f'<w:tblStyle w:val="{style}"/>'
    if bidi_visual is not None:
        properties += f'<w:bidiVisual w:val="{1 if bidi_visual else 0}"/>'
    properties += '<w:tblW w:w="0" w:type="auto"/>'

    grid = "".join('<w:gridCol w:w="2880"/>' for _ in rows[0])
    body = ""
    for row in rows:
        cells = ""
        for text in row:
            cells += (
                '<w:tc><w:tcPr><w:tcW w:w="2880" w:type="dxa"/></w:tcPr>'
                f"{paragraph(text, bidi=cell_bidi, jc=cell_jc, **(cell_run or {}))}"
                "</w:tc>"
            )
        body += f"<w:tr>{cells}</w:tr>"
    return (
        f"<w:tbl><w:tblPr>{properties}</w:tblPr>"
        f"<w:tblGrid>{grid}</w:tblGrid>{body}</w:tbl>"
    )


def document(body, *, header=True):
    """A `word/document.xml` around a body, closed by the section properties.

    `w:sectPr` is last because CT_Body puts it there, and it is where the
    header reference lives — which is how the running head becomes a part of
    this document rather than an orphan in the package.
    """
    section = "<w:sectPr>"
    if header:
        section += '<w:headerReference w:type="default" r:id="rId4"/>'
    section += (
        '<w:pgSz w:w="11906" w:h="16838"/>'
        '<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"'
        ' w:header="708" w:footer="708" w:gutter="0"/>'
        "<w:bidi/></w:sectPr>"
    )
    return (
        f"{PROLOG}<w:document xmlns:w=\"{W}\""
        f' xmlns:r="{OFFICE}/relationships">'
        f"<w:body>{body}{section}</w:body></w:document>"
    )


def header_part(text, *, bidi, jc=None, **run):
    """A running head. A header carries Arabic as readily as the body does, and
    a reader that opened `document.xml` alone would call the document clean
    while its running head ran the wrong way."""
    return (
        f'{PROLOG}<w:hdr xmlns:w="{W}" xmlns:r="{OFFICE}/relationships">'
        f"{paragraph(text, bidi=bidi, jc=jc, **run)}</w:hdr>"
    )


# ------------------------------------------------------------------ the styles


def style(style_id, name, *, kind="paragraph", based_on=None, p_props="", r_props="",
          tbl_props="", default=False):
    """One `w:style`, children in CT_Style's declared order."""
    attributes = f' w:type="{kind}" w:styleId="{style_id}"'
    if default:
        attributes += ' w:default="1"'
    body = f'<w:name w:val="{name}"/>'
    if based_on is not None:
        body += f'<w:basedOn w:val="{based_on}"/>'
    if p_props:
        body += f"<w:pPr>{p_props}</w:pPr>"
    if r_props:
        body += f"<w:rPr>{r_props}</w:rPr>"
    if tbl_props:
        body += f"<w:tblPr>{tbl_props}</w:tblPr>"
    return f"<w:style{attributes}>{body}</w:style>"


def styles_part():
    """The stylesheet both documents share.

    Three of these are load-bearing rather than decoration:

    * `Normal` states the document's right-to-left default, so a correctly
      styled Arabic paragraph that writes nothing of its own is silent — the
      false positive `Resolved` exists to prevent.
    * `EnglishBody` states the opposite under Arabic. Nobody aimed that at the
      text, so ADR 0007 reports it exactly as an absent value would be, naming
      the style.
    * `RtlTable` supplies a table's column order, which is the case that made
      a container the first unit able to inherit its direction (PLAN §3.4).
    """
    defaults = (
        "<w:docDefaults>"
        "<w:rPrDefault><w:rPr>"
        f"{r_pr(latin='Calibri', cstheme='minorBidi')}"
        "</w:rPr></w:rPrDefault>"
        "<w:pPrDefault><w:pPr></w:pPr></w:pPrDefault>"
        "</w:docDefaults>"
    )
    body = "".join(
        [
            style("Normal", "Normal", default=True,
                  p_props='<w:bidi w:val="1"/>'),
            style("Heading1", "heading 1", based_on="Normal",
                  p_props='<w:jc w:val="center"/>',
                  r_props='<w:rFonts w:cs="Dubai"/>'),
            style("EnglishBody", "English Body", based_on="Normal",
                  p_props='<w:bidi w:val="0"/>'),
            style("RtlTable", "RTL Table", kind="table",
                  tbl_props='<w:bidiVisual w:val="1"/>'),
            style("PlainTable", "Plain Table", kind="table",
                  tbl_props='<w:tblCellMar><w:top w:w="0" w:type="dxa"/>'
                            "</w:tblCellMar>"),
        ]
    )
    return f'{PROLOG}<w:styles xmlns:w="{W}">{defaults}{body}</w:styles>'


def numbering_part():
    """One bulleted list, so a real list can be told from a typed glyph."""
    level = (
        '<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/>'
        '<w:lvlText w:val="•"/><w:lvlJc w:val="left"/>'
        '<w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr>'
        '<w:rPr><w:rFonts w:ascii="Symbol" w:hAnsi="Symbol" w:hint="default"/>'
        "</w:rPr></w:lvl>"
    )
    return (
        f'{PROLOG}<w:numbering xmlns:w="{W}">'
        f'<w:abstractNum w:abstractNumId="0">'
        f'<w:multiLevelType w:val="hybridMultilevel"/>{level}</w:abstractNum>'
        f'<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>'
        "</w:numbering>"
    )


def settings_part():
    """Word's document settings.

    Present because a Word package has one and the reader enumerates every
    `word/**/*.xml`, so a part carrying no `w:p` is exactly the case that must
    produce no units. It states nothing about direction: the document-wide
    right-to-left flag lives here, and the reader does not read it, because it
    says nothing about any one paragraph.
    """
    return (
        f'{PROLOG}<w:settings xmlns:w="{W}">'
        '<w:defaultTabStop w:val="720"/>'
        "</w:settings>"
    )


# ------------------------------------------------------------- the two bodies


def defective_body():
    """Every defect the reader can find, next to text it must leave alone."""
    return "".join(
        [
            # Correct: the heading states nothing itself and is answered by a
            # chain that agrees with it. Silence here is the whole point of
            # resolving the chain at all.
            paragraph(HEADING, style="Heading1", lang="ar-SA"),
            # Nothing declared anywhere the paragraph can reach: `Normal`
            # supplies the direction, so what is left is the language and the
            # edge it starts on.
            paragraph(ARABIC),
            # The flagship: direction declared, and declared wrongly, on text
            # where that actually moves something.
            paragraph(MIXED, bidi=False, lang="ar-SA", complex_font="Dubai"),
            # A style that contradicts the Arabic under it. Reported as an
            # absent value, naming `EnglishBody` — the ADR 0007 case.
            paragraph(ARABIC, style="EnglishBody", lang="ar-SA",
                      complex_font="Dubai"),
            # A Latin font with the Arabic slot left empty: the Arabic renders
            # in whatever the application substitutes.
            paragraph(ARABIC, bidi=True, jc="end", lang="ar-SA",
                      latin="Calibri"),
            # A glyph somebody typed where a list belongs.
            paragraph(TYPED_BULLET, bidi=True, jc="end", lang="ar-SA",
                      complex_font="Dubai"),
            # Explicit bidi controls smuggled into the text.
            paragraph(CONTROLS, bidi=True, jc="end", lang="ar-SA",
                      complex_font="Dubai"),
            # Arabic pasted out of a PDF, already shaped.
            paragraph(PRESHAPED, bidi=True, jc="end", lang="ar-SA",
                      complex_font="Dubai"),
            # English, entirely undeclared, exactly as every document in the
            # world holds it. Reporting this would make the tool unusable.
            paragraph(ENGLISH, style="EnglishBody"),
            # An Arabic table whose columns run left to right, and whose cells
            # are marked correctly — so the finding lands on the table and not
            # on the text inside it.
            table(TABLE_CELLS, style="PlainTable", cell_bidi=True, cell_jc="end",
                  cell_run={"lang": "ar-SA", "complex_font": "Dubai"}),
        ]
    )


def correct_body():
    """The same document, authored the way it should have been."""
    return "".join(
        [
            paragraph(HEADING, style="Heading1", lang="ar-SA"),
            paragraph(ARABIC, bidi=True, jc="end", lang="ar-SA",
                      complex_font="Dubai"),
            paragraph(MIXED, bidi=True, jc="end", lang="ar-SA",
                      complex_font="Dubai"),
            paragraph(ARABIC, bidi=True, jc="end", lang="ar-SA",
                      complex_font="Dubai"),
            paragraph(ARABIC, bidi=True, jc="end", lang="ar-SA",
                      latin="Calibri", complex_font="Dubai"),
            # The typed glyph replaced by the list Word draws itself.
            paragraph(TYPED_BULLET.removeprefix("• "), num=True, bidi=True,
                      jc="end", lang="ar-SA", complex_font="Dubai"),
            # The controls removed; the direction states what they were faking.
            paragraph("ارتفع الأداء بنسبة 25%", bidi=True, jc="end",
                      lang="ar-SA", complex_font="Dubai"),
            # The presentation forms normalised back to logical-order Arabic.
            paragraph(ARABIC, bidi=True, jc="end", lang="ar-SA",
                      complex_font="Dubai"),
            paragraph(ENGLISH, style="EnglishBody"),
            # The table's column order supplied by the style it names, which
            # is a choice the author made and not one the tool has to report.
            table(TABLE_CELLS, style="RtlTable", cell_bidi=True, cell_jc="end",
                  cell_run={"lang": "ar-SA", "complex_font": "Dubai"}),
        ]
    )


# ---------------------------------------------------------------- the package

CONTENT_TYPES = (
    f'{PROLOG}<Types xmlns="{PKG}/content-types">'
    '<Default Extension="rels"'
    ' ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
    '<Default Extension="xml" ContentType="application/xml"/>'
    '<Override PartName="/word/document.xml" ContentType="application/vnd.'
    'openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
    '<Override PartName="/word/styles.xml" ContentType="application/vnd.'
    'openxmlformats-officedocument.wordprocessingml.styles+xml"/>'
    '<Override PartName="/word/numbering.xml" ContentType="application/vnd.'
    'openxmlformats-officedocument.wordprocessingml.numbering+xml"/>'
    '<Override PartName="/word/settings.xml" ContentType="application/vnd.'
    'openxmlformats-officedocument.wordprocessingml.settings+xml"/>'
    '<Override PartName="/word/header1.xml" ContentType="application/vnd.'
    'openxmlformats-officedocument.wordprocessingml.header+xml"/>'
    '<Override PartName="/docProps/core.xml" ContentType="application/vnd.'
    'openxmlformats-package.core-properties+xml"/>'
    '<Override PartName="/docProps/app.xml" ContentType="application/vnd.'
    'openxmlformats-officedocument.extended-properties+xml"/>'
    "</Types>"
)

ROOT_RELS = (
    f'{PROLOG}<Relationships xmlns="{PKG}/relationships">'
    f'<Relationship Id="rId1" Type="{OFFICE}/relationships/officeDocument"'
    ' Target="word/document.xml"/>'
    f'<Relationship Id="rId2" Type="{PKG}/relationships/metadata/'
    'core-properties" Target="docProps/core.xml"/>'
    f'<Relationship Id="rId3" Type="{OFFICE}/relationships/extended-properties"'
    ' Target="docProps/app.xml"/>'
    "</Relationships>"
)

DOCUMENT_RELS = (
    f'{PROLOG}<Relationships xmlns="{PKG}/relationships">'
    f'<Relationship Id="rId1" Type="{OFFICE}/relationships/styles"'
    ' Target="styles.xml"/>'
    f'<Relationship Id="rId2" Type="{OFFICE}/relationships/numbering"'
    ' Target="numbering.xml"/>'
    f'<Relationship Id="rId3" Type="{OFFICE}/relationships/settings"'
    ' Target="settings.xml"/>'
    f'<Relationship Id="rId4" Type="{OFFICE}/relationships/header"'
    ' Target="header1.xml"/>'
    "</Relationships>"
)

# Pinned, for the reason the timestamps are: a generated core property would
# put the clock into committed bytes.
CORE_PROPERTIES = (
    f'{PROLOG}<cp:coreProperties xmlns:cp="{PKG}/metadata/core-properties"'
    ' xmlns:dc="http://purl.org/dc/elements/1.1/"'
    ' xmlns:dcterms="http://purl.org/dc/terms/"'
    ' xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">'
    "<dc:title>التقرير الفصلي</dc:title>"
    "<dc:creator>mirsam corpus</dc:creator>"
    "<cp:lastModifiedBy>mirsam corpus</cp:lastModifiedBy>"
    '<dcterms:created xsi:type="dcterms:W3CDTF">2026-01-01T00:00:00Z'
    "</dcterms:created>"
    '<dcterms:modified xsi:type="dcterms:W3CDTF">2026-01-01T00:00:00Z'
    "</dcterms:modified>"
    "</cp:coreProperties>"
)

APP_PROPERTIES = (
    f'{PROLOG}<Properties xmlns="{OFFICE}/extended-properties"'
    f' xmlns:vt="{OFFICE}/docPropsVTypes">'
    "<Application>mirsam corpus generator</Application>"
    "<Pages>1</Pages><Words>0</Words><Characters>0</Characters>"
    "</Properties>"
)


def write_package(path, body, header_bidi, header_jc):
    parts = [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("word/document.xml", document(body)),
        ("word/_rels/document.xml.rels", DOCUMENT_RELS),
        ("word/styles.xml", styles_part()),
        ("word/numbering.xml", numbering_part()),
        ("word/settings.xml", settings_part()),
        (
            "word/header1.xml",
            header_part(HEADER, bidi=header_bidi, jc=header_jc, lang="ar-SA",
                        complex_font="Dubai"),
        ),
        ("docProps/core.xml", CORE_PROPERTIES),
        ("docProps/app.xml", APP_PROPERTIES),
    ]
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
        for name, text in parts:
            info = zipfile.ZipInfo(name, TIMESTAMP)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o600 << 16
            zf.writestr(info, text.encode("utf-8"))
    return [name for name, _ in parts]


def check_package(path):
    """The structural invariants `scripts/validate-ooxml.py` does not need the
    schemas to check, asserted at generation time so a broken document never
    reaches the corpus in the first place."""
    problems = []
    with zipfile.ZipFile(path) as zf:
        names = set(zf.namelist())
        declared = CONTENT_TYPES
        for name in sorted(names):
            if name == "[Content_Types].xml":
                continue
            extension = name.rsplit(".", 1)[-1].lower()
            if f'PartName="/{name}"' not in declared and extension not in {
                "rels",
                "xml",
            }:
                problems.append(f"no content type declared for {name}")

        parts = {urllib.parse.unquote(n) for n in names}
        for name in sorted(n for n in names if n.endswith(".rels")):
            source_dir = posixpath.dirname(posixpath.dirname(name))
            xml = zf.read(name).decode("utf-8")
            for chunk in xml.split('Target="')[1:]:
                target = urllib.parse.unquote(chunk.split('"')[0])
                resolved = posixpath.normpath(posixpath.join(source_dir, target))
                if resolved not in parts:
                    problems.append(f"{name}: {target} is not in the package")

        # Every `r:id` the document names has to be one the document's own
        # relationship item defines, or the header is not reachable and Word
        # opens the file without it.
        body = zf.read("word/document.xml").decode("utf-8")
        rels = zf.read("word/_rels/document.xml.rels").decode("utf-8")
        for chunk in body.split('r:id="')[1:]:
            rid = chunk.split('"')[0]
            if f'Id="{rid}"' not in rels:
                problems.append(f"word/document.xml: {rid} is not a relationship")
    return problems


def main():
    written = []
    for name, body, header_bidi, header_jc in [
        # The defective document's running head states nothing of its own and
        # is answered by `Normal`, which agrees with it — so the header is the
        # part that proves the reader looks past `document.xml` without adding
        # a finding of its own.
        ("quarterly-review.docx", defective_body(), None, "end"),
        ("quarterly-review-correct.docx", correct_body(), True, "end"),
    ]:
        path = os.path.join(OUT_DIR, name)
        parts = write_package(path, body, header_bidi, header_jc)
        problems = check_package(path)
        if problems:
            print(f"FAIL {path}", file=sys.stderr)
            for problem in problems:
                print(f"       {problem}", file=sys.stderr)
            return 1
        written.append((path, len(parts)))

    for path, count in written:
        print(f"wrote {path} ({count} parts)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
