#!/usr/bin/env python3
"""Generate the hand-built Excel corpus workbooks.

Writes two workbooks:

  tests/fixtures/quarterly-figures.xlsx
      One workbook carrying every defect the XLSX reader can find, beside text
      that is correct and text that is English — and beside the two things
      PLAN §5.3 says a repair must not disturb: a formula, and a defined name
      that points at the cell the formula lives in.

  tests/fixtures/quarterly-figures-correct.xlsx
      The same workbook authored correctly: `rightToLeft` on both sheets, an
      explicit `readingOrder` and `horizontal` on every Arabic cell, a
      complex-script typeface, a `dc:language` in the core properties, no
      typed bullet, no controls, no presentation forms and no padding.
      mirsam must leave it completely alone.

Both exercise Excel's own chain, because a workbook that never inherited
anything would let a broken `Workbook` pass: the `Normal` cell style supplies
an alignment the cells do not state, and each sheet's `rightToLeft` supplies a
reading order to the cells that state none.

Two things the defective workbook carries on purpose and the report has to say
out loud:

  * **No `dc:language`.** SpreadsheetML has no language slot on a cell, so the
    finding lands on *every* Arabic cell and the repair is refused rather than
    made — one tag answers for the whole file, and writing it to satisfy the
    Arabic would relabel the English beside it. The repetition in the report is
    the format's shape, not a bug in the corpus.
  * **A formula whose cached result is Arabic.** It produces no unit, and
    `sources.unread` names it, which is ADR 0009 reaching a second format.

Written with Python's zipfile ON PURPOSE, for the reason
`make-torture-fixture.py` gives: generating a fixture with the same `zip` crate
under test would only prove the writer agrees with itself.

Deterministic: fixed timestamps and a fixed compression level, so re-running
reproduces the committed bytes exactly.

Usage:  python3 scripts/make-excel-fixture.py [output-directory]

Then regenerate the expected reports:  make golden
"""

from __future__ import annotations

import os
import posixpath
import sys
import urllib.parse
import zipfile

OUT_DIR = sys.argv[1] if len(sys.argv) > 1 else "tests/fixtures"

# One timestamp for every entry: the corpus is committed bytes, and a generator
# that used the clock would produce a diff on every run.
TIMESTAMP = (2026, 1, 1, 0, 0, 0)

PKG = "http://schemas.openxmlformats.org/package/2006"
OFFICE = "http://schemas.openxmlformats.org/officeDocument/2006"
S = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"

PROLOG = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'

# ------------------------------------------------------------------- the text

# Mixed Arabic and Latin: the case where the base direction actually changes
# what a reader sees, because the digits and the `Q4` move.
MIXED = "ارتفع الأداء بنسبة 25% في Q4 2026."
HEADING = "ملخص تنفيذي"
ARABIC = "التقرير الفصلي للربع الرابع"
ENGLISH = "Revenue rose 25% in Q4 2026."

# Pasted out of a PDF: Arabic already shaped into presentation forms.
PRESHAPED = "ﺍﻟﺘﻘﺮﻳﺮ ﺍﻟﻔﺼﻠﻲ"

# RLE … PDF around a mixed run: the workaround this tool exists to replace.
CONTROLS = "‫ارتفع الأداء بنسبة 25%‬"

TYPED_BULLET = "• نمو في قطاع التجزئة"

# العنوان with five tatweel pushed onto the end of it, which is a heading
# stretched to a width rather than a word.
PADDED = "العنوان" + "ـ" * 5

SUMMARY_SHEET = "الملخص"
FIGURES_SHEET = "الأرقام"

HEADERS = ["المؤشر", "الربع الثالث", "الربع الرابع"]
REVENUE = "الإيرادات"
TOTAL = "الإجمالي"

FONT = "Dubai"


def escape(text: str) -> str:
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


# -------------------------------------------------------------- shared strings


class Strings:
    """`xl/sharedStrings.xml`, built as the cells are written.

    Shared rather than inline because that is what Excel writes, and because it
    is what makes the repair path in `sheet.rs` real: a repaired string is a
    new `<si>` and a repointed `<v>`, and a corpus of inline strings would
    never exercise it.
    """

    def __init__(self):
        self.items: list[str] = []
        self.index: dict[str, int] = {}
        self.references = 0

    def id_of(self, text: str) -> int:
        self.references += 1
        if text not in self.index:
            self.index[text] = len(self.items)
            self.items.append(text)
        return self.index[text]

    def part(self) -> str:
        # No `xml:space` on the `<t>`: SpreadsheetML's is a simple type with
        # no attributes at all, unlike WordprocessingML's, and `sml.xsd`
        # rejects one. Nothing here has significant leading or trailing space.
        body = "".join(f"<si><t>{escape(text)}</t></si>" for text in self.items)
        return (
            f'{PROLOG}<sst xmlns="{S}" count="{self.references}" '
            f'uniqueCount="{len(self.items)}">{body}</sst>'
        )


# --------------------------------------------------------------- the cell xfs


class Styles:
    """`xl/styles.xml`, built as the cells are written.

    A cell's formatting is not in the cell: `@s` is an index into `cellXfs`, so
    stating one cell's alignment means adding a record here. Records are shared
    between cells that want the same thing, which is exactly the sharing the
    repair path has to work around by appending rather than editing.
    """

    def __init__(self, named_alignment: str | None):
        self.named = named_alignment
        # Font 0 names no typeface; font 1 is the complex-script face the
        # correct workbook chooses.
        self.fonts = ['<font><sz val="11"/></font>', f'<font><sz val="11"/><name val="{FONT}"/></font>']
        self.records = ['<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>']
        self.index: dict[tuple, int] = {}

    def record(self, *, horizontal=None, reading_order=None, font=0) -> int:
        key = (horizontal, reading_order, font)
        if key == (None, None, 0):
            return 0
        if key in self.index:
            return self.index[key]
        alignment = ""
        if horizontal is not None:
            alignment += f' horizontal="{horizontal}"'
        if reading_order is not None:
            alignment += f' readingOrder="{reading_order}"'
        body = f"<alignment{alignment}/>" if alignment else ""
        applies = ' applyFont="1"' if font else ""
        applies += ' applyAlignment="1"' if alignment else ""
        self.records.append(
            f'<xf numFmtId="0" fontId="{font}" fillId="0" borderId="0" '
            f'xfId="0"{applies}>{body}</xf>'
            if body
            else f'<xf numFmtId="0" fontId="{font}" fillId="0" borderId="0" xfId="0"{applies}/>'
        )
        self.index[key] = len(self.records) - 1
        return self.index[key]

    def part(self) -> str:
        # The `Normal` cell style, which is what a cell's `@xfId` reaches and
        # so what supplies a value the cell's own record does not state.
        if self.named is None:
            named = '<xf numFmtId="0" fontId="0" fillId="0" borderId="0"/>'
        else:
            named = (
                '<xf numFmtId="0" fontId="0" fillId="0" borderId="0" applyAlignment="1">'
                f'<alignment horizontal="{self.named}"/></xf>'
            )
        return (
            f'{PROLOG}<styleSheet xmlns="{S}">'
            f'<fonts count="{len(self.fonts)}">{"".join(self.fonts)}</fonts>'
            '<fills count="2"><fill><patternFill patternType="none"/></fill>'
            '<fill><patternFill patternType="gray125"/></fill></fills>'
            '<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>'
            f'<cellStyleXfs count="1">{named}</cellStyleXfs>'
            f'<cellXfs count="{len(self.records)}">{"".join(self.records)}</cellXfs>'
            '<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>'
            "</styleSheet>"
        )


# -------------------------------------------------------------- the worksheets


def column_name(index: int) -> str:
    name = ""
    index += 1
    while index:
        index, remainder = divmod(index - 1, 26)
        name = chr(ord("A") + remainder) + name
    return name


def text_cell(strings, styles, row, column, text, **formatting) -> str:
    reference = f"{column_name(column)}{row}"
    style = styles.record(**formatting)
    attribute = f' s="{style}"' if style else ""
    return f'<c r="{reference}"{attribute} t="s"><v>{strings.id_of(text)}</v></c>'


def number_cell(row, column, value) -> str:
    return f'<c r="{column_name(column)}{row}"><v>{value}</v></c>'


def formula_cell(row, column, formula, cached, *, string=False, **_) -> str:
    """A cell whose value Excel recomputes on open.

    The adapter produces no unit for one — the cached value is not the
    document's text — and names it in `sources.unread` when the cache holds
    Arabic. Both halves of that are what this cell is here to prove.
    """
    reference = f"{column_name(column)}{row}"
    kind = ' t="str"' if string else ""
    return (
        f'<c r="{reference}"{kind}><f>{escape(formula)}</f>'
        f"<v>{escape(str(cached))}</v></c>"
    )


def worksheet(rows, *, right_to_left) -> str:
    views = '<sheetView workbookViewId="0"'
    if right_to_left:
        views += ' rightToLeft="1"'
    views += "/>"
    body = "".join(
        f'<row r="{index + 1}">{cells}</row>' for index, cells in enumerate(rows) if cells
    )
    return (
        f'{PROLOG}<worksheet xmlns="{S}" xmlns:r="{OFFICE}/relationships">'
        f"<sheetViews>{views}</sheetViews>"
        f'<sheetFormatPr defaultRowHeight="15"/>'
        f"<sheetData>{body}</sheetData></worksheet>"
    )


def summary(strings, styles, *, correct):
    """The first sheet: one column, so there is no column order to report on and
    every finding lands on a cell."""
    if correct:
        marked = dict(horizontal="right", reading_order="2", font=1)
        return [
            text_cell(strings, styles, 1, 0, HEADING, **marked),
            text_cell(strings, styles, 2, 0, MIXED, **marked),
            text_cell(strings, styles, 3, 0, ARABIC, **marked),
            text_cell(strings, styles, 4, 0, "نمو في قطاع التجزئة", **marked),
            text_cell(strings, styles, 5, 0, "ارتفع الأداء بنسبة 25%", **marked),
            text_cell(strings, styles, 6, 0, "التقرير الفصلي", **marked),
            text_cell(strings, styles, 7, 0, "العنوان", **marked),
            text_cell(strings, styles, 8, 0, ENGLISH),
        ]
    return [
        # Nothing declared at all: which way it reads, which edge it starts on,
        # and what language it is in are all absent.
        text_cell(strings, styles, 1, 0, HEADING),
        # Declared, and declared wrongly. The digits and the `Q4` move.
        text_cell(strings, styles, 2, 0, MIXED, horizontal="right", reading_order="1"),
        # A hard left edge under right-to-left text.
        text_cell(strings, styles, 3, 0, ARABIC, horizontal="left", reading_order="2"),
        text_cell(strings, styles, 4, 0, TYPED_BULLET, horizontal="right", reading_order="2"),
        text_cell(strings, styles, 5, 0, CONTROLS, horizontal="right", reading_order="2"),
        text_cell(strings, styles, 6, 0, PRESHAPED, horizontal="right", reading_order="2"),
        text_cell(strings, styles, 7, 0, PADDED, horizontal="right", reading_order="2"),
        # English, which the tool must not touch.
        text_cell(strings, styles, 8, 0, ENGLISH),
    ]


def figures(strings, styles, *, correct):
    """The second sheet: three columns of text, so the grid's own column order
    is a question — and the sheet that carries the formula and the cell the
    defined name points at."""
    marked = dict(horizontal="right", reading_order="2", font=1) if correct else {}
    header = "".join(
        text_cell(strings, styles, 1, column, text, **marked)
        for column, text in enumerate(HEADERS)
    )
    revenue = (
        text_cell(strings, styles, 2, 0, REVENUE, **marked)
        + number_cell(2, 1, "12.4")
        + number_cell(2, 2, "15.5")
    )
    total = (
        text_cell(strings, styles, 3, 0, TOTAL, **marked)
        + formula_cell(3, 1, "SUM(B2:B2)", "12.4")
        + formula_cell(3, 2, "SUM(C2:C2)", "15.5")
        # A formula whose cached result is Arabic: no unit, and named in
        # `sources.unread` so its absence cannot look like a clean reading.
        + formula_cell(3, 3, f"{column_name(0)}1", HEADERS[0], string=True)
    )
    return [header, revenue, total]


# ------------------------------------------------------------------ the package

CONTENT_TYPES = f"""{PROLOG}<Types xmlns="{PKG}/content-types">\
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>\
<Default Extension="xml" ContentType="application/xml"/>\
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>\
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>\
<Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>\
<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>\
<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>\
<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>\
<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>\
</Types>"""

ROOT_RELS = f"""{PROLOG}<Relationships xmlns="{PKG}/relationships">\
<Relationship Id="rId1" Type="{OFFICE}/relationships/officeDocument" Target="xl/workbook.xml"/>\
<Relationship Id="rId2" Type="{PKG}/relationships/metadata/core-properties" Target="docProps/core.xml"/>\
<Relationship Id="rId3" Type="{OFFICE}/relationships/extended-properties" Target="docProps/app.xml"/>\
</Relationships>"""

WORKBOOK_RELS = f"""{PROLOG}<Relationships xmlns="{PKG}/relationships">\
<Relationship Id="rId1" Type="{OFFICE}/relationships/worksheet" Target="worksheets/sheet1.xml"/>\
<Relationship Id="rId2" Type="{OFFICE}/relationships/worksheet" Target="worksheets/sheet2.xml"/>\
<Relationship Id="rId3" Type="{OFFICE}/relationships/styles" Target="styles.xml"/>\
<Relationship Id="rId4" Type="{OFFICE}/relationships/sharedStrings" Target="sharedStrings.xml"/>\
</Relationships>"""

# A defined name pointing into the figures sheet. Nothing a repair addresses,
# and the corpus is where "nothing addresses it" stops being a claim.
WORKBOOK = f"""{PROLOG}<workbook xmlns="{S}" xmlns:r="{OFFICE}/relationships">\
<sheets>\
<sheet name="{SUMMARY_SHEET}" sheetId="1" r:id="rId1"/>\
<sheet name="{FIGURES_SHEET}" sheetId="2" r:id="rId2"/>\
</sheets>\
<definedNames>\
<definedName name="Q4Revenue">'{FIGURES_SHEET}'!$C$2</definedName>\
<definedName name="Totals">'{FIGURES_SHEET}'!$B$3:$C$3</definedName>\
</definedNames>\
</workbook>"""

APP_PROPERTIES = f"""{PROLOG}<Properties xmlns="{OFFICE}/extended-properties" \
xmlns:vt="{OFFICE}/docPropsVTypes"><Application>mirsam corpus generator</Application>\
</Properties>"""


def core_properties(language: str | None) -> str:
    tag = f"<dc:language>{language}</dc:language>" if language else ""
    return (
        f'{PROLOG}<cp:coreProperties xmlns:cp="{PKG}/metadata/core-properties" '
        'xmlns:dc="http://purl.org/dc/elements/1.1/" '
        'xmlns:dcterms="http://purl.org/dc/terms/" '
        'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">'
        f"<dc:title>{escape(ARABIC)}</dc:title>{tag}</cp:coreProperties>"
    )


def write_package(path, *, correct):
    strings = Strings()
    # The correct workbook's `Normal` style centres what states nothing; the
    # defective one's states nothing at all, so the chain has nothing to give.
    styles = Styles("center" if correct else None)

    sheet1 = worksheet(summary(strings, styles, correct=correct), right_to_left=correct)
    sheet2 = worksheet(figures(strings, styles, correct=correct), right_to_left=correct)

    parts = [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
        ("xl/worksheets/sheet1.xml", sheet1),
        ("xl/worksheets/sheet2.xml", sheet2),
        ("xl/styles.xml", styles.part()),
        ("xl/sharedStrings.xml", strings.part()),
        ("docProps/core.xml", core_properties("ar-SA" if correct else None)),
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
    """The structural invariants asserted at generation time, so a broken
    workbook never reaches the corpus in the first place."""
    problems = []
    with zipfile.ZipFile(path) as zf:
        names = set(zf.namelist())
        for name in sorted(names):
            if name == "[Content_Types].xml":
                continue
            extension = name.rsplit(".", 1)[-1].lower()
            if f'PartName="/{name}"' not in CONTENT_TYPES and extension not in {
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

        # Every `r:id` the workbook names has to be one its own relationship
        # item defines, or the sheet is unreachable and Excel opens an empty
        # book.
        book = zf.read("xl/workbook.xml").decode("utf-8")
        rels = zf.read("xl/_rels/workbook.xml.rels").decode("utf-8")
        for chunk in book.split('r:id="')[1:]:
            rid = chunk.split('"')[0]
            if f'Id="{rid}"' not in rels:
                problems.append(f"xl/workbook.xml: {rid} is not a relationship")

        # Every `@s` a cell carries has to name a record `cellXfs` holds, and
        # every `t="s"` cell's `<v>` a string `sharedStrings.xml` holds.
        styles = zf.read("xl/styles.xml").decode("utf-8")
        records = styles.split('<cellXfs count="')[1].split('"')[0]
        strings = zf.read("xl/sharedStrings.xml").decode("utf-8")
        items = strings.split('uniqueCount="')[1].split('"')[0]
        for sheet in ("xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"):
            xml = zf.read(sheet).decode("utf-8")
            for chunk in xml.split("<c ")[1:]:
                tag = chunk.split(">")[0]
                if ' s="' in tag:
                    index = int(tag.split(' s="')[1].split('"')[0])
                    if index >= int(records):
                        problems.append(f"{sheet}: no cell format {index}")
                if 't="s"' in tag:
                    value = chunk.split("<v>")[1].split("</v>")[0]
                    if int(value) >= int(items):
                        problems.append(f"{sheet}: no shared string {value}")
    return problems


def main():
    written = []
    for name, correct in [
        ("quarterly-figures.xlsx", False),
        ("quarterly-figures-correct.xlsx", True),
    ]:
        path = os.path.join(OUT_DIR, name)
        parts = write_package(path, correct=correct)
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
