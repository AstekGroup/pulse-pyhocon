# pulse-pyhocon

**Rust/PyO3 accelerator for [pyhocon](https://github.com/chimpler/pyhocon)** — a native, **iso-functional**
HOCON parser, with **transparent fallback** to pyhocon. A *Pulse by Astek* project.

`pyhocon` is built on `pyparsing`; its parsing is dominated by interpreted-Python machinery (~99% of
the time, ~1% in C regex). `pulse-pyhocon` reimplements the hot path in Rust and delegates the rest to
pyhocon — keeping correctness, gaining an order of magnitude.

```python
import pulse_pyhocon
config = pulse_pyhocon.parse(text)   # a real pyhocon.ConfigTree (full API), parsed in Rust
config.get_int("server.port")        # get_string/int/float/bool/list/config, dotted access, defaults…
config.get("db.host", "localhost")
config.with_fallback(pulse_pyhocon.parse(defaults))
```

`parse` / `parse_string` / `parse_file` return a **real `pyhocon.ConfigTree`**, so the whole pyhocon
API works (typed getters, dotted access, `with_fallback`, `as_plain_ordered_dict`,
`HOCONConverter.to_hocon/json/yaml/properties`). `ConfigFactory`, `ConfigTree`, `HOCONConverter`,
`from_dict`, `parse_URL` are re-exported for a drop-in import. The Rust core parses quickly, then the
tree is built via `ConfigFactory.from_dict` (getters/converters stay pyhocon's → iso, no
reimplementation off the hot path).

## Why

- **~200–350× faster** than `pyhocon` for `parse(...) → ConfigTree` (the bottleneck is interpreted
  Python, not a C extension — a textbook case for Rust). Measured with a drift-immune A/B on realistic
  configs. (Raw parsing alone reaches ~1000–5000×; building the full `ConfigTree` adds a modest cost,
  kept here for full API compatibility.)
- **Iso-functional**: every output is validated against `pyhocon` by a **typed differential oracle**
  (result *and* exception type), over a large corpus + full-Unicode adversarial fuzzing.
- **Always correct**: anything the fast path does not cover is delegated **transparently** to
  `pyhocon` (internal `NotImplementedError` → fallback). The drop-in never "breaks" on HOCON that
  pyhocon accepts.

## Fast-path coverage (Rust)

Objects, arrays, scalars (int — including > 64 bits →, float, case-insensitive bool/null), dotted
keys, deep merge, comments `#`/`//`, **substitutions** `${path}`/`${?path}` (type preserved,
concatenation, forward/backward refs, sub→sub, optional omitted, env fallback), file **includes**,
object (merge) and array **concatenation**, and **self-reference** (`path = ${path}":/usr/bin"`,
`a = ${a} [2]`, object self-merge…). pyhocon exception parity (`ConfigSubstitutionException`,
`ConfigWrongTypeException`, `FileNotFoundError`).

**Delegated to the pyhocon fallback** (correct, no speedup): `+=` (pyhocon-specific behavior), quoted
keys with special characters, empty values, `include url(...)`/`classpath(...)`, a bare
`true`/`false`/`null` keyword inside an unquoted multi-token run, extreme floats in a string concat,
and the self-reference corner cases the native core does not handle (previous value not yet resolved,
absolute-path nested self-ref, path navigation through a substitution `${x.host}`). The native core
only attempts the happy path; as soon as it cannot resolve something, it delegates to pyhocon (the
oracle), which resolves the idiom or raises the right exception. **Guarantee: never a divergence**,
even on these corner cases.

## Installation

```bash
pip install pulse-pyhocon      # also pulls pyhocon (used for the fallback and the ConfigTree)
```

Prebuilt wheels are published for Linux (manylinux/musllinux x86_64 & aarch64), macOS (Apple Silicon)
and Windows. On other platforms (e.g. macOS Intel) the install builds the Rust core from the sdist (a
Rust toolchain is required); the module remains usable via the pure-Python fallback if the native
extension is unavailable.

## Status

Alpha. API: `parse`/`parse_string`/`parse_file` → `pyhocon.ConfigTree`; `parse_URL`/`from_dict`
re-exported. `pulse_pyhocon.BACKEND` is `"rust"` or `"python"`. Roadmap (perf — iso is already
guaranteed via the fallback): resolve, natively (instead of via fallback), path navigation through a
substitution; native `include url/classpath`; a macOS Intel (`universal2`) wheel.

## License & credits

Apache-2.0 (aligned with pyhocon). `pulse-pyhocon` relies on **pyhocon** (chimpler/pyhocon, Apache-2.0)
as its iso-functionality reference and as the fallback. Thanks to its authors and contributors.
