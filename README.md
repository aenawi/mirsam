# mirsam · مرسم

**Arabic text correctness for documents.** A single dependency-free binary that
finds right-to-left, bidirectional and typography defects in PowerPoint and
Word files, *proves* each one by resolving what the text will actually look
like when rendered, and repairs them without touching a byte it was not asked
to. Word is audit-only so far; the XLSX, HTML and PDF adapters are scheduled;
see [Roadmap](docs/ROADMAP.md).

> **mirsam** (مَرْسَم) — an atelier; the room where things are properly drawn.
> From the root ر-س-م: to draw, to inscribe, to render.

```console
$ mirsam audit deck.pptx
mirsam audit  deck.pptx  [pptx]
units 42 | arabic 31 | mixed 12

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
logical text       GPS يعتمد عليه النظام
dominant direction rtl
auto-detected      ltr
base direction     changes the rendering — declaring it is required
```

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
language ar-SA | font (none) | convert-bullets yes | align yes

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

Three repairs are decisions the text cannot make for you, so they are flags:
`--font <TYPEFACE>` fills the empty complex-script slot (without it the
finding is reported, not repaired); `--convert-bullets` replaces a typed
`•` with a native list (opt-in, because it edits the text itself); and
`--align` writes a start-edge alignment onto right-to-left paragraphs that
inherit one that leaves them on the left edge (opt-in, because the edge a
paragraph starts on is a design decision; without the flag that is reported
as a note, which never blocks). A paragraph whose layout centres or
right-aligns it is not reported and not touched. `--lang` changes the tag
written from `ar-SA`;
`--force` replaces an existing output. Repairing a repaired file is a no-op
that reproduces it byte for byte.

### With an AI agent

`mirsam` is designed to be called by agents. `--format json` emits the full
diagnostic model, and every finding carries evidence rather than an assertion,
so an agent can act on it without opening PowerPoint. See [`AGENTS.md`](AGENTS.md).

## Status

**v0.1 “Steppe Eagle” — audit only, PPTX only.** Byte-preserving `repair` for
PPTX has landed on `main` and ships as v0.2. A DOCX **reader** has landed too,
so `mirsam audit report.docx` works; `repair` refuses a `.docx` and says why,
because the Word writer is not built yet. The XLSX, HTML and PDF adapters are
specified and scheduled; see [`docs/ROADMAP.md`](docs/ROADMAP.md). The tool
reports what it can actually verify and nothing more — a discipline inherited
from its prior art.

## Design

Hexagonal: a pure domain that knows Unicode and knows nothing about files,
surrounded by format adapters that lower documents into a shared text model.
Read [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

Every change is checked against a golden corpus: decks under
[`tests/fixtures/`](tests/fixtures/), each with a committed report of what
`mirsam` finds in it, repairs in it and writes to it. CI fails on any
difference, so a change in behaviour is always visible against a document.

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
PowerPoint — ويُثبت كل ملاحظة عبر حساب الترتيب البصري الفعلي للنص وفق خوارزمية
يونيكود ثنائية الاتجاه.

**مرسم**: المكان الذي يُرسم فيه الشيء على وجهه الصحيح، من الجذر ر-س-م.

الحالة: الإصدار 0.1 يدعم الفحص فقط، ولملفات PowerPoint فقط. أمر الإصلاح لملفات
PowerPoint جاهز في الفرع الرئيسي وسيصدر في الإصدار 0.2، وبقية الصيغ مجدولة في
[خارطة الطريق](docs/ROADMAP.md).

الرخصة: MIT.

</div>
