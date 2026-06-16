# Upstream proposal for pyhocon (draft)

> Text ready to post as an **issue** on `chimpler/pyhocon` (Discussions are disabled there).
> **Only post it after a first `pulse-pyhocon` release on PyPI** (so the maintainer can
> `pip install pulse-pyhocon` and reproduce), and after a review. Tone: respectful, non-intrusive,
> minimal maintenance burden for them.

---

**Title**: Proposal: optional Rust-accelerated parsing backend (drop-in, ~60–2000× faster, with pure-Python fallback)

Hi @darthbear, and thanks for maintaining pyhocon 🙏

**TL;DR** — We built [`pulse-pyhocon`](https://github.com/AstekGroup/pulse-pyhocon) (on PyPI:
`pip install pulse-pyhocon`), a Rust/PyO3 parser that is **iso-functional** with pyhocon and **~60–200×
faster** while returning a real `ConfigTree` (and **~600–2000× faster** for the raw parse). It's a
*separate* package that **depends on pyhocon** and delegates anything outside its fast path back to it.
We'd love to know whether you'd be open to pyhocon *optionally* using it when present — fully opt-in,
**no impact** on users who don't install it, and **no Rust added to this repository**.

### Performance

Drift-immune A/B (median per call) on realistic configs, CPython 3.11, Apple Silicon — fully
reproducible with the snippet below:

| Config | `pyhocon.parse_string` → ConfigTree | `pulse_pyhocon.parse` → **ConfigTree** | raw Rust parse → dict |
|---|---|---|---|
| ~3 lines | 3.1 ms | **15.6 µs (≈200×)** | 1.6 µs (≈1960×) |
| ~10 lines | 8.3 ms | **112 µs (≈74×)** | 11 µs (≈744×) |
| ~14 blocks (subst-heavy) | 13.3 ms | **222 µs (≈60×)** | 21 µs (≈632×) |

The striking part: `ConfigFactory.parse_string` costs **~3 ms even for a 3-line config** — the time is
dominated by a roughly *fixed* per-call cost (building/streamlining the `pyparsing` grammar on every
call), not by the config itself. That interpreted machinery is exactly what a native parser removes.

`pulse_pyhocon.parse` returns a real `pyhocon.ConfigTree` (so the speedup includes building the tree
via `ConfigFactory.from_dict`); the "raw parse → dict" column is the parser core alone, to show the
ceiling.

```python
import statistics, time
from pyhocon import ConfigFactory
import pulse_pyhocon

cfg = open("application.conf").read()

def bench(fn, reps, rounds=15):
    out = []
    for _ in range(rounds):
        t = time.perf_counter()
        for _ in range(reps):
            fn()
        out.append((time.perf_counter() - t) / reps)
    return statistics.median(out)

ref = bench(lambda: ConfigFactory.parse_string(cfg), reps=20)
cand = bench(lambda: pulse_pyhocon.parse(cfg), reps=2000)
print(f"pyhocon {ref*1e6:.0f} us  ->  pulse {cand*1e6:.0f} us   (x{ref/cand:.0f})")
```

### Why Rust wins here (and why this is not a false friend)
Profiling a parse: ~99% of the time is in interpreted `pyparsing` (`_parseNoCache`/`parseImpl`/
`ParseResults`/grammar streamlining), ~1% in the C `re` engine. Because the hot path is *interpreted
Python* (not a C-bound regex/JSON kernel), a native rewrite wins by orders of magnitude — the opposite
of cases where rewriting only adds an FFI layer over already-native C.

### What `pulse-pyhocon` is
- A hand-written HOCON parser in Rust (PyO3/maturin, `abi3` wheels for Linux x86_64/aarch64, macOS
  Apple Silicon, Windows; sdist elsewhere).
- **Returns a real `pyhocon.ConfigTree`** — the full pyhocon API works unchanged: `get`/`get_string`/
  `get_int`/`get_float`/`get_bool`/`get_list`/`get_config`, dotted access, defaults, `with_fallback`,
  `as_plain_ordered_dict`, and `HOCONConverter.to_hocon/json/yaml/properties`. The tree is built with
  pyhocon's own `ConfigFactory.from_dict`, so getters/converters are *yours* (we don't reimplement them).
- **Covered natively**: objects, arrays, scalars (incl. arbitrary-precision ints, case-insensitive
  bool/null), dotted keys, deep merge, comments `#`/`//`, **substitutions** `${...}`/`${?...}` (type
  preserved, string concat, forward/backward refs, env fallback), **file includes**, **object/array
  concatenation**, and **self-reference** (`path = ${path}":/usr/bin"`). `parse_file`/`parse_URL`/
  `from_dict` are provided too.
- **Iso-functional**, proven by a *typed differential oracle* comparing against `ConfigFactory.parse_string`
  on **result AND exception type** — over a curated corpus **plus adversarial full-Unicode fuzzing**,
  including parity of `ConfigSubstitutionException` / `ConfigWrongTypeException` / `FileNotFoundError`.
- **Always correct**: anything outside the fast path (e.g. `+=`, `include url(...)`/`classpath(...)`,
  special quoted keys, a substitution it can't resolve identically) raises an internal
  `NotImplementedError` and is **transparently delegated to pyhocon**. It never diverges on input
  pyhocon accepts — by construction.

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
  (This deliberately avoids the hard-Rust-dependency friction seen elsewhere, e.g. the
  `jsonschema`/`rpds-py` discussion.)

### What we're asking
Just whether this direction interests you. If yes, we're happy to:
1. open a small PR wiring the optional backend behind a feature flag / env var,
2. share the differential test-suite so iso-functionality is verifiable in your CI,
3. keep `pulse-pyhocon` tracking pyhocon's behaviour as the source of truth.

If you'd rather keep pyhocon pure-Python, that's completely understandable — we'll maintain
`pulse-pyhocon` as a standalone drop-in either way, and would still value your feedback.

Repo: https://github.com/AstekGroup/pulse-pyhocon · PyPI: `pulse-pyhocon` · License: Apache-2.0 (same as pyhocon).

Thanks again!
