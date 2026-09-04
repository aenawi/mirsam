#!/usr/bin/env python3
"""Generate the hand-built PPTX test fixtures.

Writes three decks:

  tests/fixtures/torture.pptx       — the M1 round-trip acceptance deck (below)
  tests/fixtures/clean.pptx         — a correctly marked deck the tool must
                                      leave alone, so `audit` exit code 0 is
                                      provable
  tests/fixtures/broken-arabic.pptx — the M0 fixture: one slide, every defect
                                      the first rules were written against

The torture deck exists to break a naive read-modify-write cycle. It carries
every structure PLAN.md M1 1.1 names as acceptance criteria, plus the
ZIP-container variation that makes byte-preservation hard:

  * mc:AlternateContent, with mc:Ignorable referencing prefixes by name
  * an embedded chart, and the .xlsx workbook it links to (a ZIP inside a ZIP)
  * speaker notes
  * a percent-encoded ZIP item name (ppt/media/my%20image.png), which a
    rewriter that decodes names would silently rename
  * mixed compression: STORED and DEFLATED at three different levels
  * a distinct timestamp on every entry
  * XML quirks a DOM serialiser silently normalises: CRLF after the prolog,
    numeric character references, single-quoted attributes, comments,
    processing instructions, CDATA, and empty elements written both ways

All three are also *documents*, not merely containers: every part is declared
in [Content_Types].xml, every relationship resolves, and every part validates
against the ECMA-376 transitional schema for its namespace. That is not
decoration. A corpus deck that PowerPoint offers to repair cannot be used to
answer "does PowerPoint open the repaired file without a prompt", which is the
milestone's application check (#9). `check_package` below asserts the
structural half of that at generation time; `scripts/validate-ooxml.py`
asserts the schema half against the published XSDs.

Written with Python's zipfile ON PURPOSE. The round-trip test asserts that the
Rust writer reproduces this file; generating the fixture with the same `zip`
crate under test would only prove the writer agrees with itself.

Deterministic: fixed timestamps and fixed compression levels, so re-running
this script reproduces the committed bytes exactly.

Usage:  python3 scripts/make-torture-fixture.py [output-directory]
"""

import io
import os
import posixpath
import re
import struct
import sys
import urllib.parse
import zlib
import zipfile

OUT_DIR = sys.argv[1] if len(sys.argv) > 1 else "tests/fixtures"

STORED = zipfile.ZIP_STORED
DEFLATED = zipfile.ZIP_DEFLATED

# ---------------------------------------------------------------- namespaces

A = "http://schemas.openxmlformats.org/drawingml/2006/main"
P = "http://schemas.openxmlformats.org/presentationml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
MC = "http://schemas.openxmlformats.org/markup-compatibility/2006"
C = "http://schemas.openxmlformats.org/drawingml/2006/chart"
PKG_REL = "http://schemas.openxmlformats.org/package/2006/relationships"

PROLOG = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'

# Content types, by the part they belong to.
CT = {
    "presentation": "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
    "slide": "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
    "slideLayout": "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml",
    "slideMaster": "application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml",
    "notesSlide": "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml",
    "notesMaster": "application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml",
    "presProps": "application/vnd.openxmlformats-officedocument.presentationml.presProps+xml",
    "viewProps": "application/vnd.openxmlformats-officedocument.presentationml.viewProps+xml",
    "tableStyles": "application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml",
    "chart": "application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
    "theme": "application/vnd.openxmlformats-officedocument.theme+xml",
    "core": "application/vnd.openxmlformats-package.core-properties+xml",
    "app": "application/vnd.openxmlformats-officedocument.extended-properties+xml",
}

# The package-level relationship type for core properties lives in the package
# namespace, not the officeDocument one. Getting this wrong is a repair prompt.
CORE_PROPS_REL = f"{PKG_REL}/metadata/core-properties"


def png_1x1():
    """A minimal valid PNG. Already-compressed binary; must survive untouched."""

    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    ihdr = struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)
    idat = zlib.compress(b"\x00\xff\x00\x00", 9)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")


# The chart's data, in one place: the workbook holds it and the chart caches
# it. Without the cache a consumer that will not open the embedded workbook —
# Impress, among others — draws an empty plot area.
QUARTERS = ["الربع الأول", "الربع الثاني", "الربع الثالث", "الربع الرابع"]
REVENUE = [18, 21, 23, 25]


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
            f'<Relationships xmlns="{PKG_REL}">'
            f'<Relationship Id="rId1" Type="{R}/officeDocument" Target="xl/workbook.xml"/>'
            "</Relationships>"
        ))
        put("xl/workbook.xml", PROLOG + (
            f'<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="{R}">'
            '<sheets><sheet name="ورقة1" sheetId="1" r:id="rId1"/></sheets>'
            "</workbook>"
        ))
        put("xl/_rels/workbook.xml.rels", PROLOG + (
            f'<Relationships xmlns="{PKG_REL}">'
            f'<Relationship Id="rId1" Type="{R}/worksheet" Target="worksheets/sheet1.xml"/>'
            "</Relationships>"
        ))
        rows = "".join(
            f'<row r="{n}"><c r="A{n}" t="inlineStr"><is><t>{label}</t></is></c>'
            f"<c r=\"B{n}\"><v>{value}</v></c></row>"
            for n, (label, value) in enumerate(
                [("الإيرادات", 0)] + list(zip(QUARTERS, REVENUE)), start=1
            )
        )
        put("xl/worksheets/sheet1.xml", PROLOG + (
            '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
            f"<sheetData>{rows}</sheetData></worksheet>"
        ), method=STORED)
    return buf.getvalue()


# --------------------------------------------------------------- XML helpers

def rels(*entries):
    """A relationship part. Each entry is (Id, Type, Target)."""
    body = "".join(
        f'<Relationship Id="{rid}" Type="{kind}" Target="{target}"/>'
        for rid, kind, target in entries
    )
    return PROLOG + f'<Relationships xmlns="{PKG_REL}">{body}</Relationships>'


def content_types(defaults, overrides):
    body = "".join(
        f'<Default Extension="{ext}" ContentType="{ct}"/>' for ext, ct in defaults
    ) + "".join(
        f'<Override PartName="{part}" ContentType="{ct}"/>' for part, ct in overrides
    )
    return PROLOG + (
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        f"{body}</Types>"
    )


# `p:spTree` is `nvGrpSpPr` then `grpSpPr` then the shapes — both required, in
# that order. Omitting `grpSpPr` is invalid against pml.xsd, and was one of the
# reasons every hand-built deck in this corpus was rejected (#9).
SPTREE_HEAD = (
    '<p:nvGrpSpPr><p:cNvPr id="1" name="Shape Tree"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>'
    "<p:grpSpPr/>"
)


def shape_pr(x, y, cx, cy):
    """Shape properties with an explicit position, size and geometry.

    Nothing in this corpus had any. A placeholder with no `a:xfrm` anywhere in
    its slide → layout → master chain has no position to inherit, and both
    PowerPoint 2016 and Impress 25.2 render it off the left edge of the canvas
    or not at all (#9). Geometry belongs on the layout and master
    placeholders, as it does in the deck PowerPoint itself wrote; the slides
    inherit it, which is also what makes the inheritance chain M2 resolves a
    real one rather than a formality.
    """
    return (
        f'<p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>'
        '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>'
    )


# The slide is 12192000 x 6858000 EMU. Three bands, none overlapping, so a
# person opening a fixture can see at a glance whether it rendered.
TITLE_PR = shape_pr(838200, 365125, 10515600, 1325563)
BODY_PR = shape_pr(838200, 1825625, 10515600, 1325563)
# The two-column box of the one-slide decks sits in the band below the body.
COLUMNS_PR = shape_pr(838200, 3300000, 10515600, 2000000)
FRAME_OFF = '<a:off x="838200" y="3300000"/><a:ext cx="10515600" cy="3000000"/>'
# The notes page is 6858000 x 9144000, and the body sits on its lower half.
NOTES_PR = shape_pr(685800, 4571999, 5486400, 3886200)

CLR_MAP = (
    '<p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" '
    'accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" '
    'accent6="accent6" hlink="hlink" folHlink="folHlink"/>'
)


def theme(name):
    """A complete theme.

    `a:themeElements` is a sequence of three required children — `clrScheme`,
    `fontScheme`, `fmtScheme` — and each font collection needs `latin`, `ea`
    and `cs`, in that order. A theme carrying only a font scheme is invalid,
    and the theme is read eagerly when the deck is opened.
    """
    accents = "".join(
        f"<a:accent{i}><a:srgbClr val=\"{c}\"/></a:accent{i}>"
        for i, c in enumerate(
            ["4472C4", "ED7D31", "A5A5A5", "FFC000", "5B9BD5", "70AD47"], start=1
        )
    )
    fill = '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>'
    line = '<a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>'
    effect = "<a:effectStyle><a:effectLst/></a:effectStyle>"
    return PROLOG + (
        f'<a:theme xmlns:a="{A}" name="{name}">'
        "<a:themeElements>"
        '<a:clrScheme name="Office">'
        '<a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>'
        '<a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>'
        '<a:dk2><a:srgbClr val="44546A"/></a:dk2>'
        '<a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>'
        f"{accents}"
        '<a:hlink><a:srgbClr val="0563C1"/></a:hlink>'
        '<a:folHlink><a:srgbClr val="954F72"/></a:folHlink>'
        "</a:clrScheme>"
        '<a:fontScheme name="Office">'
        '<a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface=""/>'
        '<a:cs typeface="Dubai"/></a:majorFont>'
        '<a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/>'
        '<a:cs typeface="Dubai"/></a:minorFont>'
        "</a:fontScheme>"
        '<a:fmtScheme name="Office">'
        f"<a:fillStyleLst>{fill * 3}</a:fillStyleLst>"
        f"<a:lnStyleLst>{line * 3}</a:lnStyleLst>"
        f"<a:effectStyleLst>{effect * 3}</a:effectStyleLst>"
        f"<a:bgFillStyleLst>{fill * 3}</a:bgFillStyleLst>"
        "</a:fmtScheme>"
        "</a:themeElements>"
        "</a:theme>"
    )


# ------------------------------------------------------------------- XML parts

TORTURE_CONTENT_TYPES = content_types(
    [
        ("rels", "application/vnd.openxmlformats-package.relationships+xml"),
        ("xml", "application/xml"),
        ("png", "image/png"),
        ("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    ],
    [
        ("/docProps/core.xml", CT["core"]),
        ("/docProps/app.xml", CT["app"]),
        ("/ppt/presentation.xml", CT["presentation"]),
        ("/ppt/presProps.xml", CT["presProps"]),
        ("/ppt/viewProps.xml", CT["viewProps"]),
        ("/ppt/tableStyles.xml", CT["tableStyles"]),
        ("/ppt/slides/slide1.xml", CT["slide"]),
        ("/ppt/slideLayouts/slideLayout1.xml", CT["slideLayout"]),
        ("/ppt/slideMasters/slideMaster1.xml", CT["slideMaster"]),
        ("/ppt/notesSlides/notesSlide1.xml", CT["notesSlide"]),
        ("/ppt/notesMasters/notesMaster1.xml", CT["notesMaster"]),
        ("/ppt/charts/chart1.xml", CT["chart"]),
        ("/ppt/theme/theme1.xml", CT["theme"]),
        ("/ppt/theme/theme2.xml", CT["theme"]),
    ],
)

# The core and extended properties are parts like any other: they need a
# content type and a relationship from the package root, or PowerPoint sees
# two orphans.
TORTURE_ROOT_RELS = rels(
    ("rId1", f"{R}/officeDocument", "ppt/presentation.xml"),
    ("rId2", CORE_PROPS_REL, "docProps/core.xml"),
    ("rId3", f"{R}/extended-properties", "docProps/app.xml"),
)

# `p:notesMasterIdLst` follows `p:sldMasterIdLst` and precedes `p:sldIdLst`.
# A deck with a notes slide and no notes master is the shape PowerPoint asks
# to repair.
TORTURE_PRESENTATION = PROLOG + (
    f'<p:presentation xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}" saveSubsetFonts="1" rtl="1">'
    '<p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>'
    '<p:notesMasterIdLst><p:notesMasterId r:id="rId3"/></p:notesMasterIdLst>'
    '<p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst>'
    '<p:sldSz cx="12192000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/>'
    "</p:presentation>"
)

# No theme relationship here: a theme is the target of an implicit
# relationship from a *master*, never from the presentation part.
TORTURE_PRESENTATION_RELS = rels(
    ("rId1", f"{R}/slideMaster", "slideMasters/slideMaster1.xml"),
    ("rId2", f"{R}/slide", "slides/slide1.xml"),
    ("rId3", f"{R}/notesMaster", "notesMasters/notesMaster1.xml"),
    ("rId4", f"{R}/presProps", "presProps.xml"),
    ("rId5", f"{R}/viewProps", "viewProps.xml"),
    ("rId6", f"{R}/tableStyles", "tableStyles.xml"),
)

PRES_PROPS = PROLOG + f'<p:presentationPr xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"/>'

VIEW_PROPS = PROLOG + f'<p:viewPr xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"/>'

TABLE_STYLES = PROLOG + (
    f'<a:tblStyleLst xmlns:a="{A}" def="{{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}}"/>'
)

# The centrepiece. Every quirk here is one a DOM round-trip would normalise.
# The processing instruction sits at the top level, where a document-level PI
# belongs, rather than between two elements inside `p:sld`.
SLIDE1 = PROLOG + '<?mso-application progid="PowerPoint.Show"?>' + (
    f'<p:sld xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}" xmlns:mc="{MC}" '
    'xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" '
    'mc:Ignorable="p14">'
    "<!-- mc:Ignorable above names the p14 prefix as a STRING. Rename the "
    "prefix on serialisation and this document silently becomes invalid. -->"
    "<p:cSld><p:spTree>"
    + SPTREE_HEAD +
    # Shape 1: explicit LTR on RTL text -> direction-mismatch (an error)
    '<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/>'
    '<p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>'
    # Position is inherited from the layout. The long-form empty element that
    # used to be <p:spPr></p:spPr> moves to a:lstStyle, so the "written both
    # ways" hazard survives the geometry fix.
    "<p:spPr/>"
    "<p:txBody><a:bodyPr/><a:lstStyle></a:lstStyle>"
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
    f'<p:xfrm>{FRAME_OFF}</p:xfrm>'
    "<a:graphic><a:graphicData "
    'uri="http://schemas.openxmlformats.org/drawingml/2006/chart">'
    f'<c:chart xmlns:c="{C}" xmlns:r="{R}" r:id="rId2"/>'
    "</a:graphicData></a:graphic></p:graphicFrame>"
    "</mc:Choice>"
    "<mc:Fallback>"
    '<p:sp><p:nvSpPr><p:cNvPr id="4" name="Chart Fallback"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>'
    + shape_pr(838200, 3300000, 10515600, 3000000) +
    "<p:txBody><a:bodyPr/><a:lstStyle/>"
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
    "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>"
    "</p:sld>"
)

SLIDE1_RELS = rels(
    ("rId1", f"{R}/slideLayout", "../slideLayouts/slideLayout1.xml"),
    ("rId2", f"{R}/chart", "../charts/chart1.xml"),
    ("rId3", f"{R}/notesSlide", "../notesSlides/notesSlide1.xml"),
    ("rId4", f"{R}/image", "../media/my%20image.png"),
)

NOTES = PROLOG + (
    f'<p:notes xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}">'
    "<p:cSld><p:spTree>"
    + SPTREE_HEAD +
    '<p:sp><p:nvSpPr><p:cNvPr id="2" name="Notes Placeholder 1"/><p:cNvSpPr/>'
    '<p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>'
    "<p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>"
    '<a:p><a:pPr rtl="1"/><a:r><a:rPr lang="ar-SA"/>'
    "<a:t>ملاحظات المتحدث: راجع الأرقام قبل العرض.</a:t></a:r></a:p>"
    "</p:txBody></p:sp>"
    "</p:spTree></p:cSld></p:notes>"
)

NOTES_RELS = rels(
    ("rId1", f"{R}/notesMaster", "../notesMasters/notesMaster1.xml"),
    ("rId2", f"{R}/slide", "../slides/slide1.xml"),
)

NOTES_MASTER = PROLOG + (
    f'<p:notesMaster xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}">'
    "<p:cSld><p:spTree>"
    + SPTREE_HEAD +
    '<p:sp><p:nvSpPr><p:cNvPr id="2" name="Notes Placeholder"/><p:cNvSpPr/>'
    '<p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>'
    + NOTES_PR +
    "<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr/></a:p>"
    "</p:txBody></p:sp>"
    "</p:spTree></p:cSld>"
    + CLR_MAP +
    '<p:notesStyle><a:lvl1pPr rtl="1" algn="r">'
    '<a:defRPr lang="ar-SA"><a:cs typeface="Dubai"/></a:defRPr>'
    "</a:lvl1pPr></p:notesStyle>"
    "</p:notesMaster>"
)

NOTES_MASTER_RELS = rels(("rId1", f"{R}/theme", "../theme/theme2.xml"))

# A bar chart needs both of its axis identifiers and the axes they name, or
# the part does not validate and the graphic frame that references it is a
# dangling reference.
CAT_AX_ID, VAL_AX_ID = "111111111", "222222222"

def str_cache(values):
    points = "".join(
        f'<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>' for i, v in enumerate(values)
    )
    return f'<c:strCache><c:ptCount val="{len(values)}"/>{points}</c:strCache>'


def num_cache(values):
    points = "".join(
        f'<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>' for i, v in enumerate(values)
    )
    return (
        "<c:numCache><c:formatCode>General</c:formatCode>"
        f'<c:ptCount val="{len(values)}"/>{points}</c:numCache>'
    )


CHART = PROLOG + (
    f'<c:chartSpace xmlns:c="{C}" xmlns:a="{A}" xmlns:r="{R}">'
    '<c:lang val="ar-SA"/><c:roundedCorners val="0"/>'
    "<c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/>"
    '<a:p><a:pPr rtl="1"/><a:r><a:rPr lang="ar-SA"/>'
    "<a:t>الإيرادات حسب الربع</a:t></a:r></a:p>"
    "</c:rich></c:tx></c:title>"
    "<c:plotArea><c:layout/><c:barChart>"
    '<c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>'
    '<c:ser><c:idx val="0"/><c:order val="0"/>'
    "<c:tx><c:strRef><c:f>ورقة1!$A$1</c:f>"
    + str_cache(["الإيرادات"]) +
    "</c:strRef></c:tx>"
    "<c:cat><c:strRef><c:f>ورقة1!$A$2:$A$5</c:f>"
    + str_cache(QUARTERS) +
    "</c:strRef></c:cat>"
    "<c:val><c:numRef><c:f>ورقة1!$B$2:$B$5</c:f>"
    + num_cache(REVENUE) +
    "</c:numRef></c:val>"
    "</c:ser>"
    '<c:gapWidth val="150"/>'
    f'<c:axId val="{CAT_AX_ID}"/><c:axId val="{VAL_AX_ID}"/>'
    "</c:barChart>"
    f'<c:catAx><c:axId val="{CAT_AX_ID}"/>'
    '<c:scaling><c:orientation val="minMax"/></c:scaling>'
    '<c:delete val="0"/><c:axPos val="b"/>'
    f'<c:crossAx val="{VAL_AX_ID}"/></c:catAx>'
    f'<c:valAx><c:axId val="{VAL_AX_ID}"/>'
    '<c:scaling><c:orientation val="minMax"/></c:scaling>'
    '<c:delete val="0"/><c:axPos val="l"/>'
    f'<c:crossAx val="{CAT_AX_ID}"/></c:valAx>'
    '</c:plotArea><c:plotVisOnly val="1"/></c:chart>'
    '<c:externalData r:id="rId1"><c:autoUpdate val="0"/></c:externalData>'
    "</c:chartSpace>"
)

CHART_RELS = rels(
    ("rId1", f"{R}/package", "../embeddings/Microsoft_Excel_Sheet1.xlsx"),
)

LAYOUT = PROLOG + (
    f'<p:sldLayout xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}" type="title">'
    "<p:cSld><p:spTree>"
    + SPTREE_HEAD +
    '<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title Placeholder"/><p:cNvSpPr/>'
    '<p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>'
    + TITLE_PR +
    '<p:txBody><a:bodyPr/><a:lstStyle><a:lvl1pPr rtl="1" algn="r"/></a:lstStyle>'
    "<a:p><a:endParaRPr/></a:p></p:txBody></p:sp>"
    '<p:sp><p:nvSpPr><p:cNvPr id="3" name="Body Placeholder"/><p:cNvSpPr/>'
    '<p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>'
    + BODY_PR +
    '<p:txBody><a:bodyPr/><a:lstStyle><a:lvl1pPr rtl="1" algn="r"/></a:lstStyle>'
    "<a:p><a:endParaRPr/></a:p></p:txBody></p:sp>"
    "</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
)

LAYOUT_RELS = rels(
    ("rId1", f"{R}/slideMaster", "../slideMasters/slideMaster1.xml"),
)

MASTER = PROLOG + (
    f'<p:sldMaster xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}">'
    "<p:cSld><p:spTree>"
    + SPTREE_HEAD +
    '<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title Placeholder"/><p:cNvSpPr/>'
    '<p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>'
    + TITLE_PR +
    "<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr/></a:p></p:txBody></p:sp>"
    '<p:sp><p:nvSpPr><p:cNvPr id="3" name="Body Placeholder"/><p:cNvSpPr/>'
    '<p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>'
    + BODY_PR +
    "<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr/></a:p></p:txBody></p:sp>"
    "</p:spTree></p:cSld>"
    + CLR_MAP +
    '<p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst>'
    # All three styles, as a master PowerPoint wrote would have. `buNone` on
    # the body style is deliberate: without it PowerPoint supplies an
    # automatic bullet, and the typed '•' that `literal-bullet` exists to
    # catch renders as a *second* one. The defect is the character in the
    # text, so the text is where it should be visible.
    # The complex-script slot is written both ways it occurs in the wild, so
    # the corpus exercises both: `+mj-cs` / `+mn-cs` are references into the
    # theme's `a:fontScheme` (what PowerPoint itself writes) and resolve to
    # Dubai only by reading `ppt/theme/theme1.xml`; `otherStyle` names the
    # typeface outright. A deck with only the second form cannot tell a
    # resolver that reads the theme from one that treats "+mn-cs" as a font.
    '<p:txStyles><p:titleStyle><a:lvl1pPr rtl="1" algn="r">'
    '<a:defRPr lang="ar-SA"><a:cs typeface="+mj-cs"/></a:defRPr>'
    "</a:lvl1pPr></p:titleStyle>"
    '<p:bodyStyle><a:lvl1pPr rtl="1" algn="r"><a:buNone/>'
    '<a:defRPr lang="ar-SA"><a:cs typeface="+mn-cs"/></a:defRPr>'
    "</a:lvl1pPr></p:bodyStyle>"
    '<p:otherStyle><a:lvl1pPr rtl="1" algn="r">'
    '<a:defRPr lang="ar-SA"><a:cs typeface="Dubai"/></a:defRPr>'
    "</a:lvl1pPr></p:otherStyle></p:txStyles>"
    "</p:sldMaster>"
)

MASTER_RELS = rels(
    ("rId1", f"{R}/slideLayout", "../slideLayouts/slideLayout1.xml"),
    ("rId2", f"{R}/theme", "../theme/theme1.xml"),
)

THEME1 = theme("Office")
THEME2 = theme("Office Notes")

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
    "<Application><![CDATA[Microsoft Office PowerPoint]]></Application><Slides>1</Slides>"
    "</Properties>"
)

# ------------------------------------------------------------------- assembly

# (name, payload, compression method, compresslevel, date_time)
# Deliberately varied: a writer that recompresses everything at one level
# cannot reproduce this file.
ENTRIES = [
    ("[Content_Types].xml",                          TORTURE_CONTENT_TYPES,     DEFLATED, 6, (2026, 9, 2, 8, 15, 0)),
    ("_rels/.rels",                                  TORTURE_ROOT_RELS,         DEFLATED, 9, (2026, 9, 2, 8, 16, 2)),
    ("docProps/core.xml",                            CORE,                      DEFLATED, 1, (2026, 9, 2, 8, 16, 4)),
    ("docProps/app.xml",                             APP,                       STORED,   0, (2026, 9, 2, 8, 16, 6)),
    ("ppt/presentation.xml",                         TORTURE_PRESENTATION,      DEFLATED, 6, (2026, 9, 2, 8, 17, 0)),
    ("ppt/_rels/presentation.xml.rels",              TORTURE_PRESENTATION_RELS, DEFLATED, 9, (2026, 9, 2, 8, 17, 2)),
    ("ppt/presProps.xml",                            PRES_PROPS,                DEFLATED, 1, (2026, 9, 2, 8, 17, 4)),
    ("ppt/viewProps.xml",                            VIEW_PROPS,                STORED,   0, (2026, 9, 2, 8, 17, 6)),
    ("ppt/tableStyles.xml",                          TABLE_STYLES,              DEFLATED, 6, (2026, 9, 2, 8, 17, 8)),
    ("ppt/slides/slide1.xml",                        SLIDE1,                    DEFLATED, 9, (2026, 9, 2, 8, 18, 0)),
    ("ppt/slides/_rels/slide1.xml.rels",             SLIDE1_RELS,               DEFLATED, 6, (2026, 9, 2, 8, 18, 2)),
    ("ppt/slideLayouts/slideLayout1.xml",            LAYOUT,                    DEFLATED, 6, (2026, 9, 2, 8, 19, 0)),
    ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", LAYOUT_RELS,               DEFLATED, 6, (2026, 9, 2, 8, 19, 2)),
    ("ppt/slideMasters/slideMaster1.xml",            MASTER,                    DEFLATED, 1, (2026, 9, 2, 8, 20, 0)),
    ("ppt/slideMasters/_rels/slideMaster1.xml.rels", MASTER_RELS,               DEFLATED, 6, (2026, 9, 2, 8, 20, 2)),
    ("ppt/notesSlides/notesSlide1.xml",              NOTES,                     DEFLATED, 6, (2026, 9, 2, 8, 21, 0)),
    ("ppt/notesSlides/_rels/notesSlide1.xml.rels",   NOTES_RELS,                STORED,   0, (2026, 9, 2, 8, 21, 2)),
    ("ppt/notesMasters/notesMaster1.xml",            NOTES_MASTER,              DEFLATED, 6, (2026, 9, 2, 8, 21, 4)),
    ("ppt/notesMasters/_rels/notesMaster1.xml.rels", NOTES_MASTER_RELS,         DEFLATED, 9, (2026, 9, 2, 8, 21, 6)),
    ("ppt/charts/chart1.xml",                        CHART,                     DEFLATED, 9, (2026, 9, 2, 8, 22, 0)),
    ("ppt/charts/_rels/chart1.xml.rels",             CHART_RELS,                DEFLATED, 6, (2026, 9, 2, 8, 22, 2)),
    ("ppt/theme/theme1.xml",                         THEME1,                    DEFLATED, 6, (2026, 9, 2, 8, 23, 0)),
    ("ppt/theme/theme2.xml",                         THEME2,                    DEFLATED, 1, (2026, 9, 2, 8, 23, 2)),
]

BINARY_ENTRIES = [
    # A percent-encoded item name. This used to be an Arabic name, صورة.png,
    # until a 23-deck bisect on PowerPoint 2016 (#9) showed that PowerPoint
    # does not resolve a relationship to any part whose name carries a
    # non-ASCII octet — raw UTF-8 or percent-encoded, in the item name or
    # the target, Arabic or Latin — and offers to repair the deck. An
    # encoded space is accepted and keeps the hazard that matters to a
    # rewriter: a name it must copy as bytes, not decode and re-encode.
    ("ppt/media/my%20image.png", png_1x1(), STORED, 0, (2026, 9, 2, 8, 24, 0)),
    # A ZIP nested inside the ZIP. Recompressing this is both pointless and
    # detectable.
    ("ppt/embeddings/Microsoft_Excel_Sheet1.xlsx", embedded_xlsx(), STORED, 0, (2026, 9, 2, 8, 24, 2)),
]


# --------------------------------------------------- the one-slide packages

# `clean.pptx` and `broken-arabic.pptx` differ only in their slide. Both are
# smaller packages than the torture deck but not thinner ones: a master, a
# layout and a theme, so that every relationship resolves and an application
# has an inheritance chain to read. "The tool leaves a correct deck alone" and
# "the tool repairs a broken one" are only worth asserting about decks an
# application would open (#9).

ONE_SLIDE_PRESENTATION = PROLOG + (
    f'<p:presentation xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}" saveSubsetFonts="1" rtl="1">'
    '<p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>'
    '<p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst>'
    '<p:sldSz cx="12192000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/>'
    "</p:presentation>"
)

ONE_SLIDE_PRESENTATION_RELS = rels(
    ("rId1", f"{R}/slideMaster", "slideMasters/slideMaster1.xml"),
    ("rId2", f"{R}/slide", "slides/slide1.xml"),
)

ONE_SLIDE_CONTENT_TYPES = content_types(
    [
        ("rels", "application/vnd.openxmlformats-package.relationships+xml"),
        ("xml", "application/xml"),
    ],
    [
        ("/ppt/presentation.xml", CT["presentation"]),
        ("/ppt/slides/slide1.xml", CT["slide"]),
        ("/ppt/slideLayouts/slideLayout1.xml", CT["slideLayout"]),
        ("/ppt/slideMasters/slideMaster1.xml", CT["slideMaster"]),
        ("/ppt/theme/theme1.xml", CT["theme"]),
    ],
)

ONE_SLIDE_ROOT_RELS = rels(("rId1", f"{R}/officeDocument", "ppt/presentation.xml"))

ONE_SLIDE_SLIDE_RELS = rels(
    ("rId1", f"{R}/slideLayout", "../slideLayouts/slideLayout1.xml"),
)


def one_slide_deck(slide, day):
    """The entries of a single-slide package carrying `slide`.

    `day` gives the deck its own timestamps, so two decks built from this
    skeleton are still distinguishable entry by entry.
    """
    def when(minute, second):
        return (2026, 9, day, 8, minute, second)

    return [
        ("[Content_Types].xml",                          ONE_SLIDE_CONTENT_TYPES,      DEFLATED, 6, when(15, 0)),
        ("_rels/.rels",                                  ONE_SLIDE_ROOT_RELS,          DEFLATED, 6, when(15, 2)),
        ("ppt/presentation.xml",                         ONE_SLIDE_PRESENTATION,       DEFLATED, 6, when(15, 4)),
        ("ppt/_rels/presentation.xml.rels",              ONE_SLIDE_PRESENTATION_RELS,  DEFLATED, 6, when(15, 6)),
        ("ppt/slides/slide1.xml",                        slide,                        DEFLATED, 6, when(15, 8)),
        ("ppt/slides/_rels/slide1.xml.rels",             ONE_SLIDE_SLIDE_RELS,         DEFLATED, 6, when(16, 0)),
        ("ppt/slideLayouts/slideLayout1.xml",            LAYOUT,                       DEFLATED, 6, when(16, 2)),
        ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", LAYOUT_RELS,                  DEFLATED, 6, when(16, 4)),
        ("ppt/slideMasters/slideMaster1.xml",            MASTER,                       DEFLATED, 6, when(16, 6)),
        ("ppt/slideMasters/_rels/slideMaster1.xml.rels", MASTER_RELS,                  DEFLATED, 6, when(16, 8)),
        ("ppt/theme/theme1.xml",                         THEME1,                       DEFLATED, 6, when(17, 0)),
    ]


# A text body laid out in columns is a container of its own: `numCol` decides
# there are columns, `rtlCol` decides which one the reader starts in. Written
# as a pair — the correct body in the clean deck, the same body without a
# column direction in the broken one — so the corpus proves both the finding
# and the silence. The paragraphs inside are identical and correct in both,
# which is the point: a container's direction is not its paragraphs'.
def columns_box(body_pr):
    return (
        '<p:sp><p:nvSpPr><p:cNvPr id="3" name="Columns 2"/>'
        '<p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>'
        + COLUMNS_PR +
        f"<p:txBody>{body_pr}<a:lstStyle/>"
        '<a:p><a:pPr rtl="1" algn="r"/><a:r>'
        '<a:rPr lang="ar-SA"><a:cs typeface="Dubai"/></a:rPr>'
        "<a:t>العمود الأول من النص المتصل.</a:t></a:r></a:p>"
        '<a:p><a:pPr rtl="1" algn="r"/><a:r>'
        '<a:rPr lang="ar-SA"><a:cs typeface="Dubai"/></a:rPr>'
        "<a:t>العمود الثاني من النص المتصل.</a:t></a:r></a:p>"
        "</p:txBody></p:sp>"
    )


# Everything the rules ask for: an explicit base direction that matches how the
# text actually resolves, coherent alignment, an Arabic language tag, a
# complex-script font, a native bullet, no embedded controls and no
# presentation forms. `audit` must report nothing at all on this.
CLEAN_SLIDE = PROLOG + (
    f'<p:sld xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}">'
    "<p:cSld><p:spTree>"
    + SPTREE_HEAD +
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
    + columns_box('<a:bodyPr numCol="2" rtlCol="1"/>') +
    "</p:spTree></p:cSld>"
    "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>"
    "</p:sld>"
)

# The M0 fixture, and the mirror image of the one above: no direction, no
# alignment, an English language tag on Arabic text, a typed bullet and an
# embedded right-to-left mark. Every defect is in the *text*; the package
# around it is as correct as the clean deck's.
BROKEN_SLIDE = PROLOG + (
    f'<p:sld xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}">'
    "<p:cSld><p:spTree>"
    + SPTREE_HEAD +
    '<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/>'
    '<p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr/>'
    "<p:txBody><a:bodyPr/><a:lstStyle/>"
    '<a:p><a:r><a:rPr lang="en-US"/>'
    "<a:t>ارتفع الأداء 25% في Q4</a:t></a:r></a:p>"
    '<a:p><a:r><a:rPr lang="en-US"/>'
    "<a:t>• بند أول\u200f</a:t></a:r></a:p>"
    "</p:txBody></p:sp>"
    + columns_box('<a:bodyPr numCol="2"/>') +
    "</p:spTree></p:cSld>"
    "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>"
    "</p:sld>"
)

CLEAN_ENTRIES = one_slide_deck(CLEAN_SLIDE, day=2)
BROKEN_ENTRIES = one_slide_deck(BROKEN_SLIDE, day=1)


# -------------------------------------------------------------- the self-check

def check_package(path, *, allow_raw_item_names=False):
    """Assert the structural invariants an application checks on open.

    Not a schema validator — `scripts/validate-ooxml.py` is that, and it needs
    the published XSDs. This is the subset that can be asserted from the
    standard library alone, every item of it a shape that has actually made a
    deck in this corpus prompt PowerPoint to repair it.
    """
    problems = []
    with zipfile.ZipFile(path) as z:
        names = set(z.namelist())
        types = z.read("[Content_Types].xml").decode("utf-8")
        defaults = {m.lower() for m in re.findall(r'<Default Extension="([^"]+)"', types)}
        overrides = set(re.findall(r'<Override PartName="([^"]+)"', types))

        for name in sorted(names):
            if not name.isascii() and not allow_raw_item_names:
                problems.append(
                    f"{name}: a non-ASCII item name; PowerPoint 2016 does not"
                    " resolve a relationship to such a part in any encoding"
                    " and offers to repair the deck (#9)"
                )
            if name == "[Content_Types].xml":
                continue
            ext = name.rsplit(".", 1)[-1].lower() if "." in name else ""
            if "/" + name not in overrides and ext not in defaults:
                problems.append(f"no content type declared for {name}")

        related = set()
        parts = {urllib.parse.unquote(n) for n in names}
        for name in sorted(n for n in names if n.endswith(".rels")):
            source_dir = posixpath.dirname(posixpath.dirname(name))
            body = z.read(name).decode("utf-8")
            for kind, target in re.findall(
                r'<Relationship Id="[^"]*" Type="([^"]+)" Target="([^"]+)"', body
            ):
                related.add(kind)
                resolved = posixpath.normpath(
                    posixpath.join(source_dir, urllib.parse.unquote(target))
                )
                if resolved not in parts:
                    problems.append(f"{name}: {target} points at a part that is not there")

        for name in sorted(n for n in names if n.endswith(".xml")):
            body = z.read(name).decode("utf-8")
            # p:spTree is nvGrpSpPr then grpSpPr, both required.
            for tree in re.findall(r"<p:spTree>.*?</p:spTree>", body, re.S):
                if "<p:grpSpPr" not in tree:
                    problems.append(f"{name}: a p:spTree with no p:grpSpPr")
            if "<a:themeElements>" in body:
                for element in ("a:clrScheme", "a:fontScheme", "a:fmtScheme"):
                    if f"<{element}" not in body:
                        problems.append(f"{name}: a theme with no {element}")
            if "<c:barChart>" in body and body.count("<c:axId ") < 2:
                problems.append(f"{name}: a c:barChart with fewer than two c:axId")

        # A notes slide without a notes master is the deck PowerPoint repairs.
        if any(n.startswith("ppt/notesSlides/") for n in names):
            if not any(n.startswith("ppt/notesMasters/") for n in names):
                problems.append("a notes slide with no notes master part")
            presentation = z.read("ppt/presentation.xml").decode("utf-8")
            if "<p:notesMasterIdLst>" not in presentation:
                problems.append("a notes master with no p:notesMasterIdLst in presentation.xml")

        # Core properties are a part, and a part nothing relates to is invisible.
        for part, kind in (("docProps/core.xml", CORE_PROPS_REL),
                           ("docProps/app.xml", f"{R}/extended-properties")):
            if part in names and kind not in related:
                problems.append(f"{part} is in the package but nothing relates to it")

    if problems:
        raise SystemExit(
            f"{path} would not open cleanly:\n  " + "\n  ".join(problems)
        )


def write_package(path, entries, **checks):
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with zipfile.ZipFile(path, "w") as z:
        for name, payload, method, level, when in entries:
            info = zipfile.ZipInfo(name, date_time=when)
            info.compress_type = method
            info.external_attr = 0o600 << 16
            info.create_system = 0  # claim MS-DOS, as Office does
            data = payload.encode("utf-8") if isinstance(payload, str) else payload
            z.writestr(info, data, compresslevel=level if method == DEFLATED else None)
    with zipfile.ZipFile(path) as z:
        bad = z.testzip()
        assert bad is None, f"corrupt entry: {bad}"
        n = len(z.infolist())
    check_package(path, **checks)
    print(f"wrote {path}: {n} entries, {os.path.getsize(path)} bytes")


def main():
    write_package(os.path.join(OUT_DIR, "torture.pptx"), ENTRIES + BINARY_ENTRIES)
    write_package(os.path.join(OUT_DIR, "clean.pptx"), CLEAN_ENTRIES)
    write_package(os.path.join(OUT_DIR, "broken-arabic.pptx"), BROKEN_ENTRIES)


if __name__ == "__main__":
    main()
