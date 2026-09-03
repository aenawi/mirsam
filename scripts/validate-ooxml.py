#!/usr/bin/env python3
"""Validate a PPTX against the published ECMA-376 schemas.

`make-torture-fixture.py` asserts the structural invariants it can reach from
the standard library. This asserts the rest: that every XML part validates
against the ECMA-376 transitional XSD for its namespace, and that the OPC
container around them is complete.

It exists because the M1 application check — "PowerPoint opens the file
without offering to repair it" — cannot be run in CI, and a corpus deck that
fails it makes the check unanswerable
([#9](https://github.com/aenawi/mirsam/issues/9)). Schema validity is not the
same claim as "PowerPoint is happy", but it is the strongest one a machine can
make, and it is well calibrated against this corpus: the decks built on
PowerPoint's own template pass, and every defect this script reported on the
hand-built decks was a real one.

The schemas are downloaded once from ecma-international.org and cached under
`target/ooxml-schemas/`. Nothing is vendored into the repository.

Usage:
    uv run --with lxml scripts/validate-ooxml.py [deck.pptx ...]

With no arguments, every `.pptx` under `tests/fixtures/` is checked. Exits 0
when every package is clean, 1 otherwise.
"""

from __future__ import annotations

import io
import os
import posixpath
import sys
import urllib.parse
import urllib.request
import zipfile

from lxml import etree

# ECMA-376 5th edition, Part 4 — the transitional schemas. Part 1 ships the
# strict ones, which use a different namespace for every element and so cannot
# validate a document any application actually writes.
SCHEMA_URL = (
    "https://ecma-international.org/wp-content/uploads/"
    "ECMA-376-4_5th_edition_december_2016.zip"
)
INNER_ZIP = "OfficeOpenXML-XMLSchema-Transitional.zip"

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CACHE = os.path.join(ROOT, "target", "ooxml-schemas")

MC = "http://schemas.openxmlformats.org/markup-compatibility/2006"
PKG = "http://schemas.openxmlformats.org/package/2006"

# The schema that validates a part, by the namespace of its root element.
# `None` means "a package-level part the OPC checks cover instead".
SCHEMA_FOR_NS = {
    "http://schemas.openxmlformats.org/presentationml/2006/main": "pml.xsd",
    "http://schemas.openxmlformats.org/drawingml/2006/main": "dml-main.xsd",
    "http://schemas.openxmlformats.org/drawingml/2006/chart": "dml-chart.xsd",
    "http://schemas.openxmlformats.org/drawingml/2006/chartDrawing": "dml-chartDrawing.xsd",
    "http://schemas.openxmlformats.org/drawingml/2006/diagram": "dml-diagram.xsd",
    "http://schemas.openxmlformats.org/spreadsheetml/2006/main": "sml.xsd",
    "http://schemas.openxmlformats.org/wordprocessingml/2006/main": "wml.xsd",
    "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties":
        "shared-documentPropertiesExtended.xsd",
    "http://schemas.openxmlformats.org/officeDocument/2006/custom-properties":
        "shared-documentPropertiesCustom.xsd",
    f"{PKG}/relationships": None,
    f"{PKG}/content-types": None,
    f"{PKG}/metadata/core-properties": None,
    "urn:schemas-microsoft-com:vml": None,
    "urn:schemas-microsoft-com:office:office": None,
}


def schemas_dir() -> str:
    """The cached XSD directory, downloading the schema set on first use."""
    marker = os.path.join(CACHE, "pml.xsd")
    if os.path.exists(marker):
        return CACHE
    os.makedirs(CACHE, exist_ok=True)
    print(f"fetching ECMA-376 transitional schemas -> {CACHE}", file=sys.stderr)
    with urllib.request.urlopen(SCHEMA_URL, timeout=120) as response:
        outer = zipfile.ZipFile(io.BytesIO(response.read()))
    with zipfile.ZipFile(io.BytesIO(outer.read(INNER_ZIP))) as inner:
        inner.extractall(CACHE)
    return CACHE


_schemas: dict[str, etree.XMLSchema] = {}


def schema(name: str) -> etree.XMLSchema:
    if name not in _schemas:
        _schemas[name] = etree.XMLSchema(etree.parse(os.path.join(schemas_dir(), name)))
    return _schemas[name]


def apply_mce(root, branch: str):
    """Resolve Markup Compatibility so the schema sees a plain document.

    `mc:AlternateContent` collapses to one branch, and anything in a namespace
    named by `mc:Ignorable` is dropped — which is exactly what a consumer does
    before it applies the schema.
    """
    for alternate in list(root.iter(f"{{{MC}}}AlternateContent")):
        picked = alternate.find(f"{{{MC}}}{branch}")
        if picked is None:
            picked = alternate.find(f"{{{MC}}}Fallback")
        parent = alternate.getparent()
        if parent is None:
            continue
        at = list(parent).index(alternate)
        parent.remove(alternate)
        for offset, child in enumerate(list(picked) if picked is not None else []):
            parent.insert(at + offset, child)

    ignorable = {MC}
    for element in root.iter():
        if not isinstance(element.tag, str):
            continue
        declared = element.get(f"{{{MC}}}Ignorable")
        for prefix in (declared or "").split():
            if element.nsmap.get(prefix):
                ignorable.add(element.nsmap[prefix])

    def namespace_of(name: str) -> str:
        return name.split("}")[0][1:] if name.startswith("{") else ""

    for element in list(root.iter()):
        if not isinstance(element.tag, str):
            continue
        for attribute in list(element.attrib):
            if namespace_of(attribute) in ignorable:
                del element.attrib[attribute]
        if namespace_of(element.tag) in ignorable and element.getparent() is not None:
            element.getparent().remove(element)
    return root


def check_container(archive: zipfile.ZipFile) -> list[str]:
    """The OPC half: every part typed, every relationship resolving."""
    problems = []
    names = set(archive.namelist())

    if "[Content_Types].xml" not in names:
        return ["[Content_Types].xml is missing; this is not an OPC package"]

    types = etree.fromstring(archive.read("[Content_Types].xml"))
    ns = {"ct": f"{PKG}/content-types"}
    defaults = {d.get("Extension").lower() for d in types.findall("ct:Default", ns)}
    overrides = {o.get("PartName") for o in types.findall("ct:Override", ns)}

    for name in sorted(names):
        if name == "[Content_Types].xml":
            continue
        extension = name.rsplit(".", 1)[-1].lower() if "." in name else ""
        if "/" + name not in overrides and extension not in defaults:
            problems.append(f"no content type declared for {name}")

    for name in sorted(n for n in names if n.endswith(".rels")):
        source_dir = posixpath.dirname(posixpath.dirname(name))
        for relationship in etree.fromstring(archive.read(name)):
            if relationship.get("TargetMode") == "External":
                continue
            target = urllib.parse.unquote(relationship.get("Target", ""))
            resolved = posixpath.normpath(posixpath.join(source_dir, target))
            if resolved not in names:
                problems.append(f"{name}: {target} points at a part that is not there")

    return problems


def check_parts(archive: zipfile.ZipFile) -> list[str]:
    """The schema half, over both branches of every AlternateContent."""
    problems = []
    for name in sorted(archive.namelist()):
        if not (name.endswith(".xml") or name.endswith(".rels")):
            continue
        data = archive.read(name)
        try:
            root = etree.fromstring(data)
        except etree.XMLSyntaxError as error:
            problems.append(f"{name}: not well-formed: {error}")
            continue

        namespace = root.tag.split("}")[0][1:] if root.tag.startswith("{") else ""
        if namespace not in SCHEMA_FOR_NS:
            problems.append(f"{name}: no schema known for namespace {namespace}")
            continue
        xsd = SCHEMA_FOR_NS[namespace]
        if xsd is None:
            continue

        # A consumer sees one branch; validate the one PowerPoint takes and
        # the one a consumer that does not understand the requirement takes.
        for branch in ("Choice", "Fallback"):
            document = etree.ElementTree(apply_mce(etree.fromstring(data), branch))
            validator = schema(xsd)
            if not validator.validate(document):
                for error in validator.error_log:
                    problems.append(f"{name} [mc:{branch}]: {error.message}")
    return problems


def validate(path: str) -> list[str]:
    with zipfile.ZipFile(path) as archive:
        return check_container(archive) + check_parts(archive)


def main(argv: list[str]) -> int:
    paths = argv[1:]
    if not paths:
        fixtures = os.path.join(ROOT, "tests", "fixtures")
        paths = sorted(
            os.path.join(fixtures, n)
            for n in os.listdir(fixtures)
            if n.endswith(".pptx") and ".out." not in n
        )
    if not paths:
        print("no packages to validate", file=sys.stderr)
        return 1

    failed = 0
    for path in paths:
        problems = validate(path)
        name = os.path.relpath(path, ROOT)
        if problems:
            failed += 1
            print(f"FAIL {name}")
            for problem in problems:
                print(f"       {problem}")
        else:
            print(f"ok   {name}")
    print(f"\n{len(paths) - failed}/{len(paths)} package(s) valid")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
