# Admin guidance — verbatim quotes

Organiser statements from the Port Mortem 2026 **Discord**, recorded so decisions can cite them.

**Everything under "Quote" is reproduced exactly as received.** Everything under "What we take from
it" is *our reading* and carries no authority — never quote our gloss as theirs.

**Provenance limit, stated plainly:** these were relayed into the working session as text. We do not
hold message links, IDs, author handles or timestamps. If a decision is ever challenged, the quote
is a reminder of what was said, not proof — capture the permalink from Discord before relying on
either in the submission.

Recorded 2026-08-01.

---

## 1 — What the deliverable actually is

Asked by an entrant on the **JS → Go** track. Our track is JS → Rust; the answer is structural and
reads across directly.

**Their question:**

> Hello, we have chosen the track JS -> Go. But, I am facing some issues understanding, what our
> work will be.
>
> Do we have to create a JS library in Go, which will be imported and used while writing JS code.
> Are we making a library for Go, which is similar to the JS library we will choose; to be used
> while programming in Go.
>
> Apologies in advance for the silly questions.

**Admin answer, quoted:**

> Your second option, with a nuance. The deliverable is a standalone native Go package (same
> behavior as the JS lib, idiomatic Go), you're not shipping a JS dependency. But to prove
> equivalence, the JS library's original tests DO call into your Go build through a thin adapter
> (subprocess/FFI), same inputs, compare outputs. So: standalone Go lib as the product, JS tests
> calling it only as the proof harness.

### What we take from it

- **The architecture we already built is the one described.** `mnemonist-core` is the product: a
  standalone native crate, `#![forbid(unsafe_code)]`, zero dependencies, builds and tests with Node
  absent. `mnemonist-napi` is the thin adapter, existing only so the unmodified upstream tests can
  call in. We ship no JS dependency. Worth stating in the README's opening lines, now that we know
  it is the criterion.
- **"Idiomatic Go" — read across as idiomatic Rust — is a named property of the deliverable**, not
  a general code-quality nicety. That reprices every place fidelity cost us idiom *inside core*: the
  tracked `MultiSet::dimension`, `FibonacciHeap::size` as `i64` so it can go negative, the arena
  that never recycles slots, linear-scan `Set`-kind equality, error types pinned to upstream's exact
  message strings.
- **An unresolved tension, and we should not pretend otherwise.** The answer says *"same behavior as
  the JS lib"* and separately names the original tests as the proof mechanism. Those come apart
  exactly where our differential fuzzing found defects the upstream suite never exercises. If
  equivalence means *the original tests pass*, fixing untested bugs is legitimate and buys idiom for
  free. If it means *bug-for-bug*, our current approach is right and the warts are the price.
  **Worth asking them directly** — the answer decides whether a dual "pure/improved" port is needed
  at all.

---

## 2 — The Bug Catcher prize

**Admin answer, quoted:**

> The Bug Catcher prize is for a genuine bug you find in the original repo while porting it (a real
> defect in the upstream code, often surfaced when the original tests disagree with correct
> behavior). To claim it: document the bug in your submission later witin the form we share, clear
> repro steps, what the original does wrong, and how your port handles it. We review those at
> judging.

*(Typo "witin" is theirs, preserved.)*

### What we take from it

- **Claiming is a submission-form step**, reviewed at judging — not something inferred from the repo.
  A perfect `NOTES.md` that never reaches the form scores nothing.
- **The required shape is repro + what upstream does wrong + how our port handles it.** Our entries
  already carry all three, plus why the upstream suite misses it. See B-160.
- **"often surfaced when the original tests disagree with correct behavior"** is the sharpest
  ranking filter we have, and it is theirs, not ours. A defect whose upstream test *asserts* the
  wrong result proves the bug survived review; one the tests merely miss proves only that nobody
  looked. Rank for the former.
- **"how your port handles it" implies the handling is a choice.** Ours is currently "reproduced
  bug-for-bug" almost everywhere. That is defensible and deliberate — but if fixing untested bugs is
  permitted, the stronger answer is *found it, catalogued it, fixed it, and the original suite still
  passes*. Same open question as §1.
- **The 7 unverified candidates are a liability, not a bonus.** These are reviewed by judges; one
  claim disproved casts doubt across the other 68. Verify against Node 24.18.1 or demote to
  explicitly-labelled candidates. Silence is cheaper than a maybe.

---

## Open questions to put to the admins

1. Should the native port reproduce upstream bugs the original test suite does not exercise, or may
   it fix them provided that suite still passes? Reproducing costs idiomaticity in the native crate;
   fixing means the port is not bit-identical on untested inputs.
2. Is there any limit on how many Bug Catcher claims one entry may submit? We hold 57 verified
   defects and would rather submit a strong ranked subset than bury reviewers.

---

## 3 — What "demo" means, the track letter, and the video

Three further statements from the same channel, recorded 2026-08-02.

**Quoted:**

> "Real demo": no hosted instance required. For a repo port, the demo is a terminal run showing your
> port working and the original test suite passing unchanged against it. Deployment is optional, not
> expected.

> JavaScript → Rust is Track G. The FAQ is correct (F = JS→Go, G = JS→Rust). Select G. We'll fix the
> /2026 page wording later, sorry.

> Demo video (~2–5 min): show your port running, the original test suite passing unchanged against
> it, and a short walkthrough of what you ported and how you proved equivalence. It's a proof demo,
> not marketing. Upload to YouTube or Drive (public) and paste the link in the submission form we
> share in chat soon.

### What we take from it

- **The track ambiguity is resolved: G.** `.port-mortem.toml` carried a "confirm before submission"
  caveat on this from the start, because the FAQ and the /2026 page disagreed. The caveat is now
  removed and the value stands as G.
- **No deployment is needed**, which retires an open question about whether a hosted artifact was
  expected. `scripts/demo.sh` is exactly the shape asked for: a terminal run.
- **The video's three required elements** map onto the existing demo script: the port running
  (steps 3–4), the original suite passing unchanged (steps 2 and 5), and a walkthrough of what was
  ported and how equivalence was established (steps 6–9). Nothing needs inventing; the script needs
  rehearsing and narrating.
- **"A proof demo, not marketing"** matches how the script was already written — it echoes each
  command before running it, and states on screen when a step falls back rather than skipping
  silently.
- **The submission form is not yet available.** The video link, and the Bug Catcher claims from
  `docs/BUGS.md`, both go into it when it appears.
