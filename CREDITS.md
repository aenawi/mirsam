# Credits

## Prior art: `arabic-presentations` by Sultan Alsafran

`mirsam` exists because of [Sultan Alsafran](https://github.com/SultanAlsafran)'s
**Arabic Presentations** skill, published under the MIT license at
[SultanAlsafran/agent-skills](https://github.com/SultanAlsafran/agent-skills)
and on [sultanalsafran.com](https://sultanalsafran.com).

That skill was the first to state the problem clearly and to insist on the
right things:

- store Arabic in **logical Unicode order**; never reverse, never pre-shape;
- express direction through **native document properties**, never through
  invisible bidi control characters;
- treat the **editable source** as the artifact and PDF as a companion;
- report structural, visual and application QA **separately**, and record what
  was not tested as `NOT RUN` rather than inferring it.

Every one of those principles is carried forward here. The correctness contract
in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) is a restatement of his, and
the acceptance corpus in the test suite is derived from his QA reference.

**No code is shared.** `mirsam` is an independent Rust implementation with a
different architecture and a different goal: a dependency-free binary usable by
any agent or CI pipeline across many document formats, rather than a Python
helper scoped to PowerPoint inside one assistant. Where the two differ in
judgment — this tool resolves the bidirectional algorithm to prove a defect,
rather than asserting that an attribute is absent — the difference is described
in [`docs/adr/0004-prove-defects-dont-assert-them.md`](docs/adr/0004-prove-defects-dont-assert-them.md).

Thank you for publishing openly.

> شكر خاص للأستاذ **سلطان الصفران** على مهارته
> [Arabic Presentations](https://github.com/SultanAlsafran/agent-skills)،
> التي ألهمت تصميم هذه الأداة. هذا المشروع تنفيذ مستقل بلغة Rust ولا يشترك معها
> في أي شيفرة برمجية.

## Tooling

- Name selected with [`namux`](https://github.com/aenawi/namux).
- Release codenames from [`tagtastic`](https://github.com/aenawi/tagtastic), theme `arabian_birds`.
