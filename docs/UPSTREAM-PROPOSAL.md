# Proposition upstream pour pyhocon (brouillon)

> Texte prêt à poster comme **issue** sur `chimpler/pyhocon` (les Discussions y sont désactivées).
> **À ne poster qu'après un premier release de `pulse-pyhocon` sur PyPI** (pour que le mainteneur
> puisse `pip install pulse-pyhocon` et reproduire), et après relecture. Ton : respectueux, non
> intrusif, charge de maintenance minimale pour eux.

---

**Titre** : Proposal: optional Rust-accelerated parsing backend (drop-in, with pure-Python fallback)

Hi @darthbear, and thanks for maintaining pyhocon 🙏

**TL;DR** — We built `pulse-pyhocon`, a Rust/PyO3 accelerator that parses HOCON **iso-functionally**
with pyhocon and is roughly **1000–5000× faster** on parsing. It's published as a *separate* package
and **depends on pyhocon** (it delegates anything outside its fast path back to pyhocon). We'd love
to know whether you'd be open to pyhocon *optionally* using it when present — entirely opt-in, with
**zero impact** on users who don't install it and **no Rust added to this repository**.

### Why
pyhocon's parsing is dominated by the interpreted `pyparsing` machinery (profiling a ~300-char config:
~99 % of time in `_parseNoCache`/`parseImpl`/`ParseResults`/grammar streamlining, ~1 % in the C `re`
engine). That interpreted hot path is an ideal candidate for a native rewrite — and unlike a regex- or
C-bound hotspot, Rust wins big here.

### What `pulse-pyhocon` is
- A hand-written HOCON parser in Rust (PyO3/maturin, `abi3` wheels for Linux/macOS/Windows).
- **Iso-functional** with pyhocon, proven by a *differential oracle* that compares `pulse_pyhocon.parse`
  against `ConfigFactory.parse_string` (result **and** exception type) over a large corpus **plus
  adversarial full-Unicode fuzzing**. Covered: objects, arrays, scalars (incl. arbitrary-precision
  ints), dotted keys, deep merge, comments, **substitutions** `${...}`/`${?...}`, **file includes**,
  **object/array concatenation**, and parity of `ConfigSubstitutionException` / `ConfigWrongTypeException`
  / `FileNotFoundError`.
- **Always correct**: anything outside the fast path (e.g. `+=`, special quoted keys) raises an internal
  `NotImplementedError` and is **transparently delegated to pyhocon**. So it never diverges on input
  pyhocon accepts.

### The proposal (lightweight for you)
pyhocon could, at import, *optionally* route `ConfigFactory.parse_string` through `pulse_pyhocon` **iff
it's installed**, else use the current pure-Python path:

```python
try:
    import pulse_pyhocon as _accel   # optional, not a hard dependency
except ImportError:
    _accel = None
# ... use _accel.parse(...) when available, else the existing parser ...
```

- **No Rust in this repo**, no new mandatory dependency, no packaging change for pyhocon.
- Users on platforms without a wheel simply don't install the accelerator → unchanged behaviour.
  (This deliberately avoids the kind of hard-Rust-dependency friction seen elsewhere, e.g. the
  `jsonschema`/`rpds-py` discussion.)

### What we're asking
Just whether this direction interests you. If yes, we're happy to:
1. open a small PR wiring the optional backend behind a feature flag/env var,
2. share the differential test-suite so iso-functionality is verifiable in your CI,
3. keep `pulse-pyhocon` tracking pyhocon's behaviour as the source of truth.

If you'd rather keep pyhocon pure-Python, that's completely understandable — we'll maintain
`pulse-pyhocon` as a standalone drop-in either way, and would still value your feedback.

Repo: https://github.com/AstekGroup/pulse-pyhocon · License: Apache-2.0 (same as pyhocon).

Thanks again!
