#!/usr/bin/env python3
"""Bisect what still makes PowerPoint offer to repair `torture.pptx` (#9).

A diagnostic, not part of the corpus. Delete it when #9 closes.

PowerPoint 2016 on Windows 10 opens `clean.pptx` and `broken-arabic.pptx`
without a prompt and still offers to repair `torture.pptx`, so the cause is
among the things the torture deck adds to that skeleton. This writes one deck
per addition — the skeleton plus exactly one hazard — into `target/bisect/`,
so a single pass through them names every culprit rather than just the first.

Every variant is schema-valid before it is written; a prompt on an invalid
variant would say nothing. Open them in order and note which prompt:

    00-baseline   the skeleton alone            expect: no prompt
    ...
    11-torture    the whole deck                expect: prompt (today)

Usage:  python3 scripts/bisect-fixture.py [output-directory]
        uv run --with lxml scripts/validate-ooxml.py target/bisect/*.pptx
"""

import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT_DIR = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "target", "bisect")

spec = importlib.util.spec_from_file_location(
    "fixture", os.path.join(HERE, "make-torture-fixture.py")
)
f = importlib.util.module_from_spec(spec)
sys.modules["fixture"] = f
spec.loader.exec_module(f)

A, P, R, MC, C = f.A, f.P, f.R, f.MC, f.C
PROLOG, CT = f.PROLOG, f.CT
DEFLATED, STORED = f.DEFLATED, f.STORED


def slide(head="", body="", extra_ns="", shapes=""):
    """The baseline slide, with room for one hazard at a time."""
    return PROLOG + head + (
        f'<p:sld xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"{extra_ns}>'
        + body +
        "<p:cSld><p:spTree>"
        + f.SPTREE_HEAD +
        '<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/>'
        '<p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr/>'
        '<p:txBody><a:bodyPr rtlCol="1"/><a:lstStyle/>'
        '<a:p><a:pPr rtl="1" algn="r"/><a:r>'
        '<a:rPr lang="ar-SA"><a:cs typeface="Dubai"/></a:rPr>'
        "<a:t>ارتفع الأداء بنسبة 25% في Q4 2026.</a:t></a:r></a:p>"
        "</p:txBody></p:sp>"
        + shapes +
        "</p:spTree></p:cSld>"
        "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>"
        "</p:sld>"
    )


BASELINE_SLIDE = slide()


def variant(entries, replace=None, add=(), content_types=None, presentation=None,
            presentation_rels=None, slide_rels=None):
    """A copy of `entries` with parts replaced and appended."""
    replace = dict(replace or {})
    if content_types is not None:
        replace["[Content_Types].xml"] = content_types
    if presentation is not None:
        replace["ppt/presentation.xml"] = presentation
    if presentation_rels is not None:
        replace["ppt/_rels/presentation.xml.rels"] = presentation_rels
    if slide_rels is not None:
        replace["ppt/slides/_rels/slide1.xml.rels"] = slide_rels
    out = []
    for name, payload, method, level, when in entries:
        out.append((name, replace.pop(name, payload), method, level, when))
    assert not replace, f"replacement named a part that is not there: {sorted(replace)}"
    return out + list(add)


BASE = f.one_slide_deck(BASELINE_SLIDE, day=2)

BASE_TYPES = [
    ("/ppt/presentation.xml", CT["presentation"]),
    ("/ppt/slides/slide1.xml", CT["slide"]),
    ("/ppt/slideLayouts/slideLayout1.xml", CT["slideLayout"]),
    ("/ppt/slideMasters/slideMaster1.xml", CT["slideMaster"]),
    ("/ppt/theme/theme1.xml", CT["theme"]),
]
BASE_DEFAULTS = [
    ("rels", "application/vnd.openxmlformats-package.relationships+xml"),
    ("xml", "application/xml"),
]


def types(defaults=(), overrides=()):
    return f.content_types(BASE_DEFAULTS + list(defaults), BASE_TYPES + list(overrides))


def at(minute, second):
    return (2026, 9, 2, minute, second, 0)


# ------------------------------------------------------------------- variants

VARIANTS = {}

VARIANTS["00-baseline"] = BASE

# 1. A document-level processing instruction in a slide part.
VARIANTS["01-pi"] = variant(
    BASE, replace={"ppt/slides/slide1.xml": slide(head='<?mso-application progid="PowerPoint.Show"?>')}
)

# 2. An XML comment inside p:sld, before p:cSld.
VARIANTS["02-comment"] = variant(
    BASE,
    replace={"ppt/slides/slide1.xml": slide(body="<!-- a comment a DOM round-trip would drop -->")},
)

# 3. Numeric character references and a single-quoted attribute.
QUIRKY_SLIDE = PROLOG + (
    f'<p:sld xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}">'
    "<p:cSld><p:spTree>"
    + f.SPTREE_HEAD +
    '<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/>'
    '<p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr/>'
    '<p:txBody><a:bodyPr rtlCol="1"/><a:lstStyle/>'
    "<a:p><a:pPr rtl=\"1\" algn='r'/><a:r>"
    '<a:rPr lang="ar-SA"><a:cs typeface="Dubai"/></a:rPr>'
    "<a:t>&#1585;&#1587;&#1605; &#1576;&#1610;&#1575;&#1606;&#1610;</a:t>"
    "</a:r></a:p>"
    "</p:txBody></p:sp>"
    "</p:spTree></p:cSld>"
    "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>"
    "</p:sld>"
)
VARIANTS["03-charrefs"] = variant(BASE, replace={"ppt/slides/slide1.xml": QUIRKY_SLIDE})

# 4. mc:AlternateContent and mc:Ignorable, with no chart inside.
MCE_SHAPE = (
    "<mc:AlternateContent "
    'xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main">'
    '<mc:Choice Requires="a14">'
    '<p:sp><p:nvSpPr><p:cNvPr id="3" name="Choice"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>'
    + f.shape_pr(838200, 3300000, 10515600, 3000000) +
    '<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:pPr rtl="1"/><a:r>'
    '<a:rPr lang="ar-SA"/><a:t>خيار</a:t></a:r></a:p></p:txBody></p:sp>'
    "</mc:Choice><mc:Fallback>"
    '<p:sp><p:nvSpPr><p:cNvPr id="4" name="Fallback"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>'
    + f.shape_pr(838200, 3300000, 10515600, 3000000) +
    '<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:pPr rtl="1"/><a:r>'
    '<a:rPr lang="ar-SA"/><a:t>بديل</a:t></a:r></a:p></p:txBody></p:sp>'
    "</mc:Fallback></mc:AlternateContent>"
)
VARIANTS["04-altcontent"] = variant(
    BASE,
    replace={
        "ppt/slides/slide1.xml": slide(
            extra_ns=(
                f' xmlns:mc="{MC}"'
                ' xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main"'
                ' mc:Ignorable="p14"'
            ),
            shapes=MCE_SHAPE,
        )
    },
)

# 5. Speaker notes: the notes slide, its master, and that master's theme.
VARIANTS["05-notes"] = variant(
    BASE,
    presentation=f.TORTURE_PRESENTATION.replace(
        '<p:notesMasterId r:id="rId3"/>', '<p:notesMasterId r:id="rId3"/>'
    ),
    presentation_rels=f.rels(
        ("rId1", f"{R}/slideMaster", "slideMasters/slideMaster1.xml"),
        ("rId2", f"{R}/slide", "slides/slide1.xml"),
        ("rId3", f"{R}/notesMaster", "notesMasters/notesMaster1.xml"),
    ),
    slide_rels=f.rels(
        ("rId1", f"{R}/slideLayout", "../slideLayouts/slideLayout1.xml"),
        ("rId3", f"{R}/notesSlide", "../notesSlides/notesSlide1.xml"),
    ),
    content_types=types(
        overrides=[
            ("/ppt/notesSlides/notesSlide1.xml", CT["notesSlide"]),
            ("/ppt/notesMasters/notesMaster1.xml", CT["notesMaster"]),
            ("/ppt/theme/theme2.xml", CT["theme"]),
        ]
    ),
    add=[
        ("ppt/notesSlides/notesSlide1.xml", f.NOTES, DEFLATED, 6, at(21, 0)),
        ("ppt/notesSlides/_rels/notesSlide1.xml.rels", f.NOTES_RELS, DEFLATED, 6, at(21, 2)),
        ("ppt/notesMasters/notesMaster1.xml", f.NOTES_MASTER, DEFLATED, 6, at(21, 4)),
        ("ppt/notesMasters/_rels/notesMaster1.xml.rels", f.NOTES_MASTER_RELS, DEFLATED, 6, at(21, 6)),
        ("ppt/theme/theme2.xml", f.THEME2, DEFLATED, 6, at(23, 2)),
    ],
)

# 6. The chart, the graphic frame that shows it, and the workbook behind it.
CHART_FRAME = (
    "<p:graphicFrame><p:nvGraphicFramePr>"
    '<p:cNvPr id="3" name="Chart 2"/><p:cNvGraphicFramePr/><p:nvPr/>'
    "</p:nvGraphicFramePr>"
    f"<p:xfrm>{f.FRAME_OFF}</p:xfrm>"
    '<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">'
    f'<c:chart xmlns:c="{C}" xmlns:r="{R}" r:id="rId2"/>'
    "</a:graphicData></a:graphic></p:graphicFrame>"
)
VARIANTS["06-chart"] = variant(
    BASE,
    replace={"ppt/slides/slide1.xml": slide(shapes=CHART_FRAME)},
    slide_rels=f.rels(
        ("rId1", f"{R}/slideLayout", "../slideLayouts/slideLayout1.xml"),
        ("rId2", f"{R}/chart", "../charts/chart1.xml"),
    ),
    content_types=types(
        defaults=[("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")],
        overrides=[("/ppt/charts/chart1.xml", CT["chart"])],
    ),
    add=[
        ("ppt/charts/chart1.xml", f.CHART, DEFLATED, 6, at(22, 0)),
        ("ppt/charts/_rels/chart1.xml.rels", f.CHART_RELS, DEFLATED, 6, at(22, 2)),
        ("ppt/embeddings/Microsoft_Excel_Sheet1.xlsx", f.embedded_xlsx(), STORED, 0, at(24, 2)),
    ],
)

# 7. A part whose name is not ASCII.
VARIANTS["07-media"] = variant(
    BASE,
    slide_rels=f.rels(
        ("rId1", f"{R}/slideLayout", "../slideLayouts/slideLayout1.xml"),
        ("rId4", f"{R}/image", "../media/%D8%B5%D9%88%D8%B1%D8%A9.png"),
    ),
    content_types=types(defaults=[("png", "image/png")]),
    add=[("ppt/media/صورة.png", f.png_1x1(), DEFLATED, 6, at(24, 0))],
)

# 8. Core and extended properties, the second carrying a CDATA section.
VARIANTS["08-docprops"] = variant(
    BASE,
    replace={"_rels/.rels": f.TORTURE_ROOT_RELS},
    content_types=types(
        overrides=[("/docProps/core.xml", CT["core"]), ("/docProps/app.xml", CT["app"])]
    ),
    add=[
        ("docProps/core.xml", f.CORE, DEFLATED, 6, at(16, 4)),
        ("docProps/app.xml", f.APP, DEFLATED, 6, at(16, 6)),
    ],
)

# 9. The three presentation-level property parts.
VARIANTS["09-presprops"] = variant(
    BASE,
    presentation_rels=f.rels(
        ("rId1", f"{R}/slideMaster", "slideMasters/slideMaster1.xml"),
        ("rId2", f"{R}/slide", "slides/slide1.xml"),
        ("rId4", f"{R}/presProps", "presProps.xml"),
        ("rId5", f"{R}/viewProps", "viewProps.xml"),
        ("rId6", f"{R}/tableStyles", "tableStyles.xml"),
    ),
    content_types=types(
        overrides=[
            ("/ppt/presProps.xml", CT["presProps"]),
            ("/ppt/viewProps.xml", CT["viewProps"]),
            ("/ppt/tableStyles.xml", CT["tableStyles"]),
        ]
    ),
    add=[
        ("ppt/presProps.xml", f.PRES_PROPS, DEFLATED, 6, at(17, 4)),
        ("ppt/viewProps.xml", f.VIEW_PROPS, DEFLATED, 6, at(17, 6)),
        ("ppt/tableStyles.xml", f.TABLE_STYLES, DEFLATED, 6, at(17, 8)),
    ],
)

# 10. The same parts, stored uncompressed and at mixed deflate levels.
VARIANTS["10-stored"] = [
    (name, payload, STORED if i % 3 == 0 else DEFLATED, 0 if i % 3 == 0 else (1 + (i % 9)), when)
    for i, (name, payload, _, _, when) in enumerate(BASE)
]

# 11. The whole deck, as committed.
VARIANTS["11-torture"] = f.ENTRIES + f.BINARY_ENTRIES


def main():
    out = os.path.abspath(OUT_DIR)
    os.makedirs(out, exist_ok=True)
    for name, entries in VARIANTS.items():
        f.write_package(os.path.join(out, f"{name}.pptx"), entries)
    print(f"\n{len(VARIANTS)} variants in {out}")


if __name__ == "__main__":
    main()
