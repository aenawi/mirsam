# Bisect decks for #9

**Temporary.** These exist to answer one open question, and should be deleted
with `scripts/bisect-fixture.py` when it is answered.

## The question

PowerPoint 2016 on Windows 10 still offers to repair `tests/fixtures/torture.pptx`,
after the fixes in [#15](https://github.com/aenawi/mirsam/pull/15) stopped
`clean.pptx` and `broken-arabic.pptx` from prompting. So the cause is among the
things the torture deck adds on top of that now-clean skeleton — and it is not
something a machine here can see: every deck below validates against the
ECMA-376 transitional schemas (`make validate-fixtures`), and LibreOffice
Impress opens all of them without a prompt.

Schema validity turned out to be **necessary but not sufficient**. The only
instrument left is a person in front of an application.

## The decks

Each is the known-good skeleton **plus exactly one** addition, so a single pass
names *every* culprit rather than stopping at the first. Regenerate with
`make bisect`; they are deterministic, so a deck that comes out different is a
real change.

| deck | the one thing it adds |
|---|---|
| `00-baseline` | nothing — control, should not prompt |
| `01-pi` | `<?mso-application?>` in a slide part |
| `02-comment` | an XML comment inside `p:sld` |
| `03-charrefs` | numeric character references, single-quoted attribute |
| `04-altcontent` | `mc:AlternateContent` / `mc:Ignorable` naming a prefix |
| `05-notes` | notes slide + notes master + that master's theme |
| `06-chart` | the chart, its graphic frame, the `.xlsx` inside the `.pptx` |
| `07-media` | a non-ASCII part name, `ppt/media/صورة.png` |
| `08-docprops` | `docProps/core.xml` + `app.xml`, the second carrying CDATA |
| `09-presprops` | `presProps` / `viewProps` / `tableStyles` |
| `10-stored` | uncompressed entries and mixed deflate levels |
| `11-torture` | the whole deck — control, prompts today |

`06-chart` and `07-media` are the standing suspects.

## What to record

For each deck: **does it prompt to repair**, and **does it render** — text on
the canvas, right-to-left where the deck says so, the chart drawn. A rendering
fault is worth reporting even where there is no prompt; the missing `a:xfrm`
and the chart with no cached data were both found that way, by eye, after the
schemas had said the decks were fine.

Note the application and the OS. PowerPoint's tolerance differs by version and
platform, and that difference is itself a finding.

## Results so far

`y` = prompts to repair, `n` = does not, `·` = not yet run.

| deck | PowerPoint 2016 / Win 10 | PowerPoint / macOS | Impress 25.2 / Ubuntu 24.04 |
|---|---|---|---|
| `00-baseline` | · | · | · |
| `01-pi` | · | · | · |
| `02-comment` | · | · | · |
| `03-charrefs` | · | · | · |
| `04-altcontent` | · | · | · |
| `05-notes` | · | · | · |
| `06-chart` | · | · | · |
| `07-media` | · | · | · |
| `08-docprops` | · | · | · |
| `09-presprops` | · | · | · |
| `10-stored` | · | · | · |
| `11-torture` | **y** | · | n |

For reference, the corpus decks themselves, same session:

| deck | PowerPoint 2016 / Win 10 | Impress 25.2 / Ubuntu 24.04 |
|---|---|---|
| `torture.pptx` | **y** | n |
| `torture.out.pptx` | **y** | n |
| `clean.pptx` | n | n |
| `broken-arabic.pptx` | n | n |
| `broken-arabic.out.pptx` | n | n |
| `quarterly-report.pptx` | n | — |

Fill the table in and put it on #9.
