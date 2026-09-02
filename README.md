# mirsam · مرسم

**Arabic text correctness for documents.** A single dependency-free binary that
finds right-to-left, bidirectional and typography defects in PowerPoint files —
and *proves* each one by resolving what the text will actually look like when
rendered. Repair, and the DOCX, XLSX, HTML and PDF adapters, are scheduled; see
[Roadmap](docs/ROADMAP.md).

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

Requires Rust 1.85 or later. `cargo install mirsam-cli` from crates.io, and
prebuilt binaries for Linux, macOS and Windows, are milestone M7 — see
[`docs/ROADMAP.md`](docs/ROADMAP.md).

## Use

```bash
mirsam audit deck.pptx                  # inspect; exit 1 if blocking
mirsam audit deck.pptx --strict         # warnings block too
mirsam audit deck.pptx --format json    # for agents and CI
mirsam explain "<text>"                 # reproduce a defect without a document
mirsam rules                            # every check, with its id
```

Exit codes are stable: `0` clean, `1` findings, `2` bad invocation,
`3` unreadable document.

### With an AI agent

`mirsam` is designed to be called by agents. `--format json` emits the full
diagnostic model, and every finding carries evidence rather than an assertion,
so an agent can act on it without opening PowerPoint. See [`AGENTS.md`](AGENTS.md).

## Status

**v0.1 “Steppe Eagle” — audit only, PPTX only.** `repair` and the DOCX, XLSX,
HTML and PDF adapters are specified and scheduled; see
[`docs/ROADMAP.md`](docs/ROADMAP.md). The tool reports what it can actually
verify and nothing more — a discipline inherited from its prior art.

## Design

Hexagonal: a pure domain that knows Unicode and knows nothing about files,
surrounded by format adapters that lower documents into a shared text model.
Read [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

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

الحالة: الإصدار 0.1 يدعم الفحص فقط، ولملفات PowerPoint فقط. بقية الصيغ وأمر
الإصلاح مجدولة في [خارطة الطريق](docs/ROADMAP.md).

الرخصة: MIT.

</div>
