#!/usr/bin/env python3
"""Generate the PPTX test fixtures.

Writes two decks:

  tests/fixtures/torture.pptx  — the M1 round-trip acceptance deck (below)
  tests/fixtures/clean.pptx    — a correctly marked deck the tool must leave
                                 alone, so `audit` exit code 0 is provable

This deck exists to break a naive read-modify-write cycle. It carries every
structure PLAN.md M1 1.1 names as acceptance criteria, plus the ZIP-container
variation that makes byte-preservation hard:

  * mc:AlternateContent, with mc:Ignorable referencing prefixes by name
  * an embedded chart, and the .xlsx workbook it links to (a ZIP inside a ZIP)
  * speaker notes
  * a non-ASCII part name (ppt/media/صورة.png)
  * mixed compression: STORED and DEFLATED at three different levels
  * a distinct timestamp on every entry
  * XML quirks a DOM serialiser silently normalises: CRLF after the prolog,
    numeric character references, single-quoted attributes, comments,
    processing instructions, CDATA, and empty elements written both ways

Written with Python's zipfile ON PURPOSE. The round-trip test asserts that the
Rust writer reproduces this file; generating the fixture with the same `zip`
crate under test would only prove the writer agrees with itself.

Deterministic: fixed timestamps and fixed compression levels, so re-running
this script reproduces the committed bytes exactly.

Usage:  python3 scripts/make-torture-fixture.py [output.pptx]
"""

import io
import os
import struct
import sys
import zlib
import zipfile

OUT = sys.argv[1] if len(sys.argv) > 1 else "tests/fixtures/torture.pptx"
CLEAN_OUT = sys.argv[2] if len(sys.argv) > 2 else "tests/fixtures/clean.pptx"

STORED = zipfile.ZIP_STORED
DEFLATED = zipfile.ZIP_DEFLATED

# ---------------------------------------------------------------- namespaces

A = "http://schemas.openxmlformats.org/drawingml/2006/main"
P = "http://schemas.openxmlformats.org/presentationml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
MC = "http://schemas.openxmlformats.org/markup-compatibility/2006"
C = "http://schemas.openxmlformats.org/drawingml/2006/chart"

PROLOG = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'


def png_1x1():
    """A minimal valid PNG. Already-compressed binary; must survive untouched."""

    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    ihdr = struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)
    idat = zlib.compress(b"\x00\xff\x00\x00", 9)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")


def embedded_xlsx():
    """A tiny but structurally real .xlsx — a ZIP nested inside the PPTX."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        def put(name, text, method=DEFLATED):
            info = zipfile.ZipInfo(name, date_time=(2026, 9, 2, 8, 15, 0))
            info.compress_type = method
            info.external_attr = 0o600 << 16
            z.writestr(info, text)

        put("[Content_Types].xml", PROLOG + (
            '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="xml" ContentType="application/xml"/>'
            '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
            '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
            "</Types>"
        ))
        put("_rels/.rels", PROLOG + (
            f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            f'<Relationship Id="rId1" Type="{R}/officeDocument" Target="xl/workbook.xml"/>'
            "</Relationships>"
        ))
        put("xl/workbook.xml", PROLOG + (
            f'<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="{R}">'
            '<sheets><sheet name="ورقة1" sheetId="1" r:id="rId1"/></sheets>'
            "</workbook>"
        ))
        put("xl/_rels/workbook.xml.rels", PROLOG + (
            f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            f'<Relationship Id="rId1" Type="{R}/worksheet" Target="worksheets/sheet1.xml"/>'
            "</Relationships>"
        ))
        put("xl/worksheets/sheet1.xml", PROLOG + (
            '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
            '<sheetData>'
            '<row r="1"><c r="A1" t="inlineStr"><is><t>الربع الرابع</t></is></c>'
            '<c r="B1"><v>25</v></c></row>'
            "</sheetData></worksheet>"
        ), method=STORED)
    return buf.getvalue()


# ------------------------------------------------------------------- XML parts

CONTENT_TYPES = PROLOG + (
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
    '<Default Extension="xml" ContentType="application/xml"/>'
    '<Default Extension="png" ContentType="image/png"/>'
    '<Default Extension="xlsx" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"/>'
    '<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>'
    '<Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>'
    '<Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/>'
    '<Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>'
    '<Override PartName="/ppt/notesSlides/notesSlide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml"/>'
    '<Override PartName="/ppt/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>'
    '<Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>'
    "</Types>"
)

ROOT_RELS = PROLOG + (
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    f'<Relationship Id="rId1" Type="{R}/officeDocument" Target="ppt/presentation.xml"/>'
    "</Relationships>"
)

PRESENTATION = PROLOG + (
    f'<p:presentation xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}" saveSubsetFonts="1" rtl="1">'
    '<p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>'
    '<p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst>'
    '<p:sldSz cx="12192000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/>'
    "</p:presentation>"
)

PRESENTATION_RELS = PROLOG + (
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    f'<Relationship Id="rId1" Type="{R}/slideMaster" Target="slideMasters/slideMaster1.xml"/>'
    f'<Relationship Id="rId2" Type="{R}/slide" Target="slides/slide1.xml"/>'
    f'<Relationship Id="rId3" Type="{R}/theme" Target="theme/theme1.xml"/>'
    "</Relationships>"
)

# The centrepiece. Every quirk here is one a DOM round-trip would normalise.
SLIDE1 = PROLOG + (
    f'<p:sld xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}" xmlns:mc="{MC}" '
    'xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" '
    'mc:Ignorable="p14">'
    "<!-- mc:Ignorable above names the p14 prefix as a STRING. Rename the "
    "prefix on serialisation and this document silently becomes invalid. -->"
    "<p:cSld><p:spTree>"
    '<p:nvGrpSpPr><p:cNvPr id="1" name="Shape Tree"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>'
    # Shape 1: explicit LTR on RTL text -> direction-mismatch (an error)
    '<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/>'
    '<p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>'
    "<p:spPr></p:spPr>"
    "<p:txBody><a:bodyPr/><a:lstStyle/>"
    '<a:p><a:pPr rtl="0" algn=\'l\'/>'
    '<a:r><a:rPr lang="en-US" dirty="0"/>'
    "<a:t>ارتفع الأداء بنسبة 25% في Q4 2026.</a:t></a:r></a:p>"
    "</p:txBody></p:sp>"
    # Shape 2: an AlternateContent block wrapping the graphic frame
    "<mc:AlternateContent "
    'xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main">'
    '<mc:Choice Requires="a14">'
    '<p:graphicFrame><p:nvGraphicFramePr>'
    '<p:cNvPr id="3" name="Chart 2"/><p:cNvGraphicFramePr/><p:nvPr/>'
    "</p:nvGraphicFramePr>"
    '<p:xfrm><a:off x="838200" y="1825625"/><a:ext cx="10515600" cy="4351338"/></p:xfrm>'
    "<a:graphic><a:graphicData "
    'uri="http://schemas.openxmlformats.org/drawingml/2006/chart">'
    f'<c:chart xmlns:c="{C}" xmlns:r="{R}" r:id="rId2"/>'
    "</a:graphicData></a:graphic></p:graphicFrame>"
    "</mc:Choice>"
    "<mc:Fallback>"
    '<p:sp><p:nvSpPr><p:cNvPr id="4" name="Chart Fallback"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>'
    "<p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>"
    # Numeric character references: same text, different bytes.
    "<a:p><a:r><a:t>&#1585;&#1587;&#1605; &#1576;&#1610;&#1575;&#1606;&#1610;</a:t></a:r></a:p>"
    "</p:txBody></p:sp>"
    "</mc:Fallback>"
    "</mc:AlternateContent>"
    # Shape 3: a typed bullet and an embedded RLM -> two more findings
    '<p:sp><p:nvSpPr><p:cNvPr id="5" name="Body 3"/><p:cNvSpPr/>'
    '<p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>'
    "<p:spPr/><p:txBody><a:bodyPr rtlCol=\"1\"/><a:lstStyle/>"
    '<a:p><a:r><a:rPr lang="en-US"/><a:t>• بند أول‏</a:t></a:r></a:p>'
    "</p:txBody></p:sp>"
    "</p:spTree></p:cSld>"
    '<?mso-application progid="PowerPoint.Slide"?>'
    "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>"
    "</p:sld>"
)

SLIDE1_RELS = PROLOG + (
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    f'<Relationship Id="rId1" Type="{R}/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>'
    f'<Relationship Id="rId2" Type="{R}/chart" Target="../charts/chart1.xml"/>'
    f'<Relationship Id="rId3" Type="{R}/notesSlide" Target="../notesSlides/notesSlide1.xml"/>'
    f'<Relationship Id="rId4" Type="{R}/image" Target="../media/%D8%B5%D9%88%D8%B1%D8%A9.png"/>'
    "</Relationships>"
)

NOTES = PROLOG + (
    f'<p:notes xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}">'
    "<p:cSld><p:spTree>"
    '<p:nvGrpSpPr><p:cNvPr id="1" name="Shape Tree"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>'
    '<p:sp><p:nvSpPr><p:cNvPr id="2" name="Notes Placeholder 1"/><p:cNvSpPr/>'
    '<p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>'
    "<p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>"
    '<a:p><a:pPr rtl="1"/><a:r><a:rPr lang="ar-SA"/>'
    "<a:t>ملاحظات المتحدث: راجع الأرقام قبل العرض.</a:t></a:r></a:p>"
    "</p:txBody></p:sp>"
    "</p:spTree></p:cSld></p:notes>"
)

NOTES_RELS = PROLOG + (
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    f'<Relationship Id="rId1" Type="{R}/slide" Target="../slides/slide1.xml"/>'
    "</Relationships>"
)

CHART = PROLOG + (
    f'<c:chartSpace xmlns:c="{C}" xmlns:a="{A}" xmlns:r="{R}">'
    "<c:lang val=\"ar-SA\"/><c:roundedCorners val=\"0\"/>"
    "<c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/>"
    '<a:p><a:pPr rtl="1"/><a:r><a:rPr lang="ar-SA"/>'
    "<a:t>الإيرادات حسب الربع</a:t></a:r></a:p>"
    "</c:rich></c:tx></c:title>"
    "<c:plotArea><c:layout/><c:barChart>"
    '<c:barDir val="col"/><c:grouping val="clustered"/>'
    "<c:ser><c:idx val=\"0\"/><c:order val=\"0\"/>"
    "<c:tx><c:strRef><c:f>ورقة1!$A$1</c:f></c:strRef></c:tx>"
    "</c:ser></c:barChart></c:plotArea></c:chart>"
    "<c:externalData r:id=\"rId1\"><c:autoUpdate val=\"0\"/></c:externalData>"
    "</c:chartSpace>"
)

CHART_RELS = PROLOG + (
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    f'<Relationship Id="rId1" Type="{R}/package" '
    'Target="../embeddings/Microsoft_Excel_Sheet1.xlsx"/>'
    "</Relationships>"
)

LAYOUT = PROLOG + (
    f'<p:sldLayout xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}" type="title">'
    "<p:cSld><p:spTree>"
    '<p:nvGrpSpPr><p:cNvPr id="1" name="Shape Tree"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>'
    '<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title Placeholder"/><p:cNvSpPr/>'
    '<p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>'
    '<p:spPr/><p:txBody><a:bodyPr/><a:lstStyle><a:lvl1pPr rtl="1" algn="r"/></a:lstStyle>'
    "<a:p><a:endParaRPr/></a:p></p:txBody></p:sp>"
    "</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
)

LAYOUT_RELS = PROLOG + (
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    f'<Relationship Id="rId1" Type="{R}/slideMaster" Target="../slideMasters/slideMaster1.xml"/>'
    "</Relationships>"
)

MASTER = PROLOG + (
    f'<p:sldMaster xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}">'
    "<p:cSld><p:spTree>"
    '<p:nvGrpSpPr><p:cNvPr id="1" name="Shape Tree"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>'
    "</p:spTree></p:cSld>"
    "<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" "
    "accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" "
    "accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>"
    '<p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst>'
    '<p:txStyles><p:titleStyle><a:lvl1pPr rtl="1" algn="r">'
    '<a:defRPr lang="ar-SA"><a:cs typeface="Dubai"/></a:defRPr>'
    "</a:lvl1pPr></p:titleStyle></p:txStyles>"
    "</p:sldMaster>"
)

MASTER_RELS = PROLOG + (
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    f'<Relationship Id="rId1" Type="{R}/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>'
    f'<Relationship Id="rId2" Type="{R}/theme" Target="../theme/theme1.xml"/>'
    "</Relationships>"
)

THEME = PROLOG + (
    f'<a:theme xmlns:a="{A}" name="Office">'
    "<a:themeElements><a:fontScheme name=\"Office\">"
    '<a:majorFont><a:latin typeface="Calibri Light"/><a:cs typeface="Dubai"/></a:majorFont>'
    '<a:minorFont><a:latin typeface="Calibri"/><a:cs typeface="Dubai"/></a:minorFont>'
    "</a:fontScheme></a:themeElements>"
    "</a:theme>"
)

CORE = PROLOG + (
    '<cp:coreProperties '
    'xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" '
    'xmlns:dc="http://purl.org/dc/elements/1.1/" '
    'xmlns:dcterms="http://purl.org/dc/terms/" '
    'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">'
    "<dc:title>عرض تجريبي</dc:title>"
    "<dc:creator>mirsam</dc:creator>"
    '<dcterms:created xsi:type="dcterms:W3CDTF">2026-09-02T08:15:00Z</dcterms:created>'
    "</cp:coreProperties>"
)

APP = PROLOG + (
    '<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties">'
    "<Application>Microsoft Office PowerPoint</Application><Slides>1</Slides>"
    "<![CDATA[]]>"
    "</Properties>"
)

# ------------------------------------------------------------------- assembly

# (name, payload, compression method, compresslevel, date_time)
# Deliberately varied: a writer that recompresses everything at one level
# cannot reproduce this file.
ENTRIES = [
    ("[Content_Types].xml",                       CONTENT_TYPES,   DEFLATED, 6, (2026, 9, 2, 8, 15, 0)),
    ("_rels/.rels",                               ROOT_RELS,       DEFLATED, 9, (2026, 9, 2, 8, 16, 2)),
    ("docProps/core.xml",                         CORE,            DEFLATED, 1, (2026, 9, 2, 8, 16, 4)),
    ("docProps/app.xml",                          APP,             STORED,   0, (2026, 9, 2, 8, 16, 6)),
    ("ppt/presentation.xml",                      PRESENTATION,    DEFLATED, 6, (2026, 9, 2, 8, 17, 0)),
    ("ppt/_rels/presentation.xml.rels",           PRESENTATION_RELS, DEFLATED, 9, (2026, 9, 2, 8, 17, 2)),
    ("ppt/slides/slide1.xml",                     SLIDE1,          DEFLATED, 9, (2026, 9, 2, 8, 18, 0)),
    ("ppt/slides/_rels/slide1.xml.rels",          SLIDE1_RELS,     DEFLATED, 6, (2026, 9, 2, 8, 18, 2)),
    ("ppt/slideLayouts/slideLayout1.xml",         LAYOUT,          DEFLATED, 6, (2026, 9, 2, 8, 19, 0)),
    ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", LAYOUT_RELS,  DEFLATED, 6, (2026, 9, 2, 8, 19, 2)),
    ("ppt/slideMasters/slideMaster1.xml",         MASTER,          DEFLATED, 1, (2026, 9, 2, 8, 20, 0)),
    ("ppt/slideMasters/_rels/slideMaster1.xml.rels", MASTER_RELS,  DEFLATED, 6, (2026, 9, 2, 8, 20, 2)),
    ("ppt/notesSlides/notesSlide1.xml",           NOTES,           DEFLATED, 6, (2026, 9, 2, 8, 21, 0)),
    ("ppt/notesSlides/_rels/notesSlide1.xml.rels", NOTES_RELS,     STORED,   0, (2026, 9, 2, 8, 21, 2)),
    ("ppt/charts/chart1.xml",                     CHART,           DEFLATED, 9, (2026, 9, 2, 8, 22, 0)),
    ("ppt/charts/_rels/chart1.xml.rels",          CHART_RELS,      DEFLATED, 6, (2026, 9, 2, 8, 22, 2)),
    ("ppt/theme/theme1.xml",                      THEME,           DEFLATED, 6, (2026, 9, 2, 8, 23, 0)),
]

BINARY_ENTRIES = [
    # Non-ASCII part name: "صورة" is Arabic for "image". Forces the UTF-8
    # general-purpose flag (bit 11) in the local header.
    ("ppt/media/صورة.png", png_1x1(), STORED, 0, (2026, 9, 2, 8, 24, 0)),
    # A ZIP nested inside the ZIP. Recompressing this is both pointless and
    # detectable.
    ("ppt/embeddings/Microsoft_Excel_Sheet1.xlsx", embedded_xlsx(), STORED, 0, (2026, 9, 2, 8, 24, 2)),
]


# ------------------------------------------------------------- the clean deck

# Everything the eight rules ask for: an explicit base direction that matches
# how the text actually resolves, coherent alignment, an Arabic language tag, a
# complex-script font, a native bullet, no embedded controls and no
# presentation forms. `audit` must report nothing at all on this.
CLEAN_SLIDE = PROLOG + (
    f'<p:sld xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}">'
    "<p:cSld><p:spTree>"
    '<p:nvGrpSpPr><p:cNvPr id="1" name="Shape Tree"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>'
    '<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/>'
    '<p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr/>'
    '<p:txBody><a:bodyPr rtlCol="1"/><a:lstStyle/>'
    '<a:p><a:pPr rtl="1" algn="r"/><a:r>'
    '<a:rPr lang="ar-SA"><a:cs typeface="Dubai"/></a:rPr>'
    "<a:t>ارتفع الأداء بنسبة 25% في Q4 2026.</a:t></a:r></a:p>"
    '<a:p><a:pPr rtl="1" algn="r"><a:buChar char="•"/></a:pPr><a:r>'
    '<a:rPr lang="ar-SA"><a:cs typeface="Dubai"/></a:rPr>'
    "<a:t>بند أول</a:t></a:r></a:p>"
    "</p:txBody></p:sp>"
    "</p:spTree></p:cSld></p:sld>"
)

CLEAN_CONTENT_TYPES = PROLOG + (
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
    '<Default Extension="xml" ContentType="application/xml"/>'
    '<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>'
    '<Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>'
    "</Types>"
)

CLEAN_ENTRIES = [
    ("[Content_Types].xml", CLEAN_CONTENT_TYPES, DEFLATED, 6, (2026, 9, 2, 8, 15, 0)),
    ("_rels/.rels", ROOT_RELS, DEFLATED, 6, (2026, 9, 2, 8, 15, 2)),
    ("ppt/presentation.xml", PRESENTATION, DEFLATED, 6, (2026, 9, 2, 8, 15, 4)),
    ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS, DEFLATED, 6, (2026, 9, 2, 8, 15, 6)),
    ("ppt/slides/slide1.xml", CLEAN_SLIDE, DEFLATED, 6, (2026, 9, 2, 8, 15, 8)),
]


def write_package(path, entries):
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with zipfile.ZipFile(path, "w") as z:
        for name, payload, method, level, when in entries:
            info = zipfile.ZipInfo(name, date_time=when)
            info.compress_type = method
            info.external_attr = 0o600 << 16
            info.create_system = 0
            data = payload.encode("utf-8") if isinstance(payload, str) else payload
            z.writestr(info, data, compresslevel=level if method == DEFLATED else None)
    with zipfile.ZipFile(path) as z:
        bad = z.testzip()
        assert bad is None, f"corrupt entry: {bad}"
        n = len(z.infolist())
    print(f"wrote {path}: {n} entries, {os.path.getsize(path)} bytes")


def main():
    os.makedirs(os.path.dirname(OUT) or ".", exist_ok=True)
    with zipfile.ZipFile(OUT, "w") as z:
        for name, payload, method, level, when in ENTRIES:
            info = zipfile.ZipInfo(name, date_time=when)
            info.compress_type = method
            info.external_attr = 0o600 << 16
            info.create_system = 0  # claim MS-DOS, as Office does
            z.writestr(info, payload.encode("utf-8"),
                       compresslevel=level if method == DEFLATED else None)
        for name, payload, method, level, when in BINARY_ENTRIES:
            info = zipfile.ZipInfo(name, date_time=when)
            info.compress_type = method
            info.external_attr = 0o600 << 16
            info.create_system = 0
            z.writestr(info, payload,
                       compresslevel=level if method == DEFLATED else None)

    size = os.path.getsize(OUT)
    with zipfile.ZipFile(OUT) as z:
        bad = z.testzip()
        assert bad is None, f"corrupt entry: {bad}"
        n = len(z.infolist())
    print(f"wrote {OUT}: {n} entries, {size} bytes")

    write_package(CLEAN_OUT, CLEAN_ENTRIES)


if __name__ == "__main__":
    main()
