# mirsam · مرسم

**Arabic text correctness for documents.** A single dependency-free binary that
finds right-to-left, bidirectional and typography defects in PowerPoint, Word,
Excel and HTML files, *proves* each one by resolving what the text will
actually look like when rendered, and repairs them without touching a byte it
was not asked to. PowerPoint, Word and Excel are repaired; HTML is audit-only
so far, and the PDF adapter is scheduled; see [Roadmap](docs/ROADMAP.md).

> **mirsam** (مَرْسَم) — an atelier; the room where things are properly drawn.
> From the root ر-س-م: to draw, to inscribe, to render.

```console
$ mirsam audit deck.pptx
mirsam audit  deck.pptx  [pptx]
units 42 | arabic 31 | mixed 12
fonts NOT RUN — pass --fonts to resolve each typeface here and report Arabic it cannot draw or cannot join

error   [direction-mismatch] ppt/slides/slide4.xml:paragraph-2:Body 3
        renders as ltr but reads as rtl; visual order differs from the logical text
        text: ارتفع الأداء بنسبة 25% في Q4 2026.

FAIL: errors=1 warnings=3 notes=0 strict=no
```

## Why another tool

Existing Arabic linters check whether an attribute is present. That produces
false positives on every paragraph that legitimately *inherits* its direction
from a layout, and false negatives on text that is mis-tagged but happens to
render correctly anyway.

`mirsam` runs the Unicode bidirectional algorithm (UAX#9) over the actual
string and reports a defect only when the resolved visual order is wrong:

```console
$ mirsam explain "GPS يعتمد عليه النظام"
logical text      GPS يعتمد عليه النظام
dominant direction rtl
auto-detected      ltr
base direction     changes the rendering — declaring it is required
  as rtl           \u{0645}\u{0627}\u{0638}\u{0646}\u{0644}\u{0627} \u{0647}\u{064A}\u{0644}\u{0639} \u{062F}\u{0645}\u{062A}\u{0639}\u{064A} GPS
  as ltr           GPS \u{0645}\u{0627}\u{0638}\u{0646}\u{0644}\u{0627} \u{0647}\u{064A}\u{0644}\u{0639} \u{062F}\u{0645}\u{062A}\u{0639}\u{064A}
```

The last two lines are the resolved *visual* orders, printed as codepoints
rather than glyphs: a terminal re-applies the bidi algorithm to text that has
already been reordered, so the escapes are what can be trusted.

That is the difference between *"the RTL flag is missing"* and *"this sentence
will render with the acronym on the wrong side, here is the proof."*

## Install

Not published yet. Build from source — a Rust toolchain is the only thing
needed, and what comes out is one binary with no runtime, no interpreter and no
package manager:

```bash
git clone https://github.com/aenawi/mirsam.git
cd mirsam
cargo install --path crates/mirsam-cli
```

Requires Rust 1.88 or later. `cargo install mirsam-cli` from crates.io, and
prebuilt binaries for Linux, macOS and Windows, are milestone M7 — see
[`docs/ROADMAP.md`](docs/ROADMAP.md).

## Use

```bash
mirsam audit deck.pptx                  # inspect; exit 1 if blocking
mirsam audit deck.pptx --strict         # warnings block too
mirsam audit deck.pptx --format json    # for agents and CI
mirsam audit deck.pptx --fonts          # also check the fonts installed here
mirsam repair deck.pptx fixed.pptx      # write a repaired copy, then audit it
mirsam explain "<text>"                 # reproduce a defect without a document
mirsam rules                            # every check, with its id
```

Exit codes are stable: `0` clean, `1` findings, `2` bad invocation,
`3` unreadable document (or, for `repair`, unwritable output).

### Repair

`repair` never modifies its input and never overwrites it, whatever flags are
given. It writes a copy in which only the attributes a finding named have
changed — every other byte of every other part is copied across
already-compressed — then re-reads that copy and audits it, so the report
describes the file on disk rather than the intention:

```console
$ mirsam repair deck.pptx fixed.pptx --convert-bullets --align
mirsam repair  deck.pptx -> fixed.pptx  [pptx]
units 2 | arabic 2 | mixed 1
language ar-SA | font (none) | convert-bullets yes | strip-tatweel no | align yes
fonts NOT RUN — pass --fonts to resolve each typeface here and report Arabic it cannot draw or cannot join

applied 8 repair(s)
  ppt/slides/slide1.xml:paragraph-1:Title 1
    set direction rtl
    set alignment start
    set language ar-SA
  ppt/slides/slide1.xml:paragraph-2:Title 1
    remove 1 explicit bidi control(s)
    set direction rtl
    set alignment start
    set language ar-SA
    convert typed '•' to a native bullet

before  errors=1 warnings=5 notes=2
after   errors=0 warnings=0 notes=0

PASS: errors=0 warnings=0 notes=0 strict=no
```

Four repairs are decisions the text cannot make for you, so they are flags:
`--font <TYPEFACE>` fills the empty complex-script slot (without it the
finding is reported, not repaired); `--convert-bullets` replaces a typed
`•` with a native list (opt-in, because it edits the text itself);
`--strip-tatweel` deletes typed tatweel that pads a heading to a width
(opt-in for the same reason, and more so — it deletes characters somebody
typed); and
`--align` writes a start-edge alignment onto right-to-left paragraphs that
inherit one that leaves them on the left edge (opt-in, because the edge a
paragraph starts on is a design decision; without the flag that is reported
as a note, which never blocks). A paragraph whose layout centres or
right-aligns it is not reported and not touched. `--lang` changes the tag
written from `ar-SA`;
`--force` replaces an existing output. Repairing a repaired file is a no-op
that reproduces it byte for byte.

### Checking the fonts

`--fonts` adds two checks the rest of the tool cannot make from the file
alone. Each paragraph's complex-script typeface is resolved against the fonts
installed on this machine, and the Arabic is put through a real OpenType
shaper: `font-coverage` reports the exact characters the font has no glyph for
— they render as empty boxes — and `shaping-broken` reports a font that has
every letter and no shaping tables, which renders Arabic as a row of
disconnected letters. `--font-dir <DIR>` searches given directories instead of
the platform's, which is how you make the answer reproducible.

It is opt-in because it reads the machine rather than the document, and an
audit that resolved fonts unasked would report differently on two computers
looking at one file. Every report says which of the two audits it is —
`fonts NOT RUN`, or `"fonts": {"checked": false}` — so their silence is never
mistaken for a pass.

Read the result one way only. A font that is *here* and cannot draw the text
will not draw it anywhere; a font that is here and draws it perfectly proves
nothing about anyone else's machine.

### With an AI agent

`mirsam` is designed to be called by agents. `--format json` emits the full
diagnostic model, and every finding carries evidence rather than an assertion,
so an agent can act on it without opening PowerPoint. See [`AGENTS.md`](AGENTS.md).

## Status

**On `main` today: four formats read, three of them repaired.** `audit` reads
`.pptx`, `.docx`, `.xlsx` and `.html`; byte-preserving `repair` writes `.pptx`,
`.docx` and `.xlsx`, and refuses `.html` rather than approximating it. Shaping
and font-coverage checks are behind `--fonts`. Milestones M0 through M5 are
complete; PDF (M6) and distribution (M7) are not — see
[`docs/ROADMAP.md`](docs/ROADMAP.md).

Nothing is tagged or published yet. `VERSION` reads `0.1.0` — the “Steppe
Eagle” foundation, which was PPTX audit only — and every version number in the
roadmap is a plan rather than a release. What you get is what you build from
`main`, which is the whole list above.

HTML is a **reader**: `mirsam audit page.html` reads `dir`, `lang` and the part
of CSS that decides direction — including a stylesheet the page links by a
relative path. `repair` refuses it and says why, because its writer is not
built. A page linking a stylesheet on a server is audited without it and the
report names what it did not read, because this tool makes no network calls.

XLSX is the second format with a **writer**: `mirsam repair book.xlsx
fixed.xlsx` sets a sheet's `rightToLeft` and a cell's reading order and
alignment, and leaves every formula and defined name exactly as it found them
— a workbook's formatting is shared between its cells, so a repair appends a
format record and repoints the one cell rather than editing what forty cells
are using. A formula's cached result is not judged and is named in the report,
for the reason a stylesheet on a server is.

DOCX is the third: `mirsam repair report.docx fixed.docx` writes `w:bidi`,
`w:jc`, the complex-script language and font slots, a table's `w:bidiVisual`
and a real list in place of a typed bullet — each in the schema position Word
requires. Two repairs are refused rather than approximated, and both refusals
are the format's: Word has no way to state a *physical* edge, and a typed
bullet can only join a list the document already defines. The PDF adapter is
specified and scheduled; see [`docs/ROADMAP.md`](docs/ROADMAP.md). The tool
reports what it can actually verify and nothing more — a discipline inherited
from its prior art.

## Design

Hexagonal: a pure domain that knows Unicode and knows nothing about files,
surrounded by format adapters that lower documents into a shared text model.
Read [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

Every change is checked against a golden corpus: presentations and documents
under [`tests/fixtures/`](tests/fixtures/), each with a committed report of
what `mirsam` finds in it, repairs in it and writes to it. CI fails on any
difference, so a change in behaviour is always visible against a document.
One conformance suite runs the same cases against every adapter, so the three
formats cannot drift into three rule sets.

## Credits

Inspired by the **Arabic Presentations** skill by
[Sultan Alsafran](https://github.com/SultanAlsafran). Independent
implementation, no shared code. See [`CREDITS.md`](CREDITS.md).

## License

MIT. See [`LICENSE`](LICENSE).

---

<div dir="rtl">

# مرسم

**أداة لضبط صحّة النصوص العربية في المستندات.** ملف تنفيذي واحد بلا اعتماديات،
يكتشف مشاكل الاتجاه من اليمين إلى اليسار والنص ثنائي الاتجاه والطباعة في ملفات
PowerPoint وWord وExcel وHTML — ويُثبت كل ملاحظة عبر حساب الترتيب البصري الفعلي
للنص وفق خوارزمية يونيكود ثنائية الاتجاه، ثم يُصلحها دون المساس ببايت واحد لم
يُطلب منه تغييره.

**مرسم**: المكان الذي يُرسم فيه الشيء على وجهه الصحيح، من الجذر ر-س-م.

الحالة على الفرع الرئيسي: الأمر `audit` يقرأ أربع صيغ — `.pptx` و`.docx`
و`.xlsx` و`.html` — والأمر `repair` يكتب ثلاثًا منها: `.pptx` و`.docx`
و`.xlsx`. أما `.html` فيُرفض إصلاحه صراحةً لأن كاتبه لم يُبنَ بعد، ولا يُقارَب
تقريبًا. وفحص التشكيل الطباعي وتغطية الخطوط متاح عبر الخيار `--fonts`. صيغة PDF
مجدولة للقراءة فقط، ولن تُصلَح أبدًا في مكانها؛ انظر
[خارطة الطريق](docs/ROADMAP.md).

لم يُوسم أي إصدار ولم يُنشر بعد. ملف `VERSION` يحمل الرقم 0.1.0 — أساس «Steppe
Eagle» الذي كان يدعم فحص PowerPoint فقط — وكل رقم إصدار في خارطة الطريق خطّة لا
إصدارًا فعليًا. فما تحصل عليه هو ما تبنيه من الفرع الرئيسي، وهو كل ما سبق.

الرخصة: MIT.

</div>
