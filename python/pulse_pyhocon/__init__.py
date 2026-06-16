"""Rust/PyO3 drop-in for pyhocon — full API, returning a real `ConfigTree`.

`parse`/`parse_string`/`parse_file` return a `pyhocon.ConfigTree`, so the whole pyhocon API works
(`get`, `get_string`/`get_int`/`get_float`/`get_bool`/`get_list`/`get_config`, dotted access, defaults,
`with_fallback`, `as_plain_ordered_dict`, `HOCONConverter.to_hocon/json/yaml/properties`…).

Fast path: the Rust core parses quickly (flat dict), which we wrap into a `ConfigTree` via
`ConfigFactory.from_dict` (pyhocon's canonical dict→tree builder → iso, verified on structure and
getters). We do NOT reimplement the getters/converters in Rust (they are not the hot path): we reuse
pyhocon's on the tree. Anything outside the fast path, or any resolution failure, raises
`NotImplementedError` → transparent delegation to pyhocon (the oracle). Iso guaranteed.

PULSE_FORCE_FALLBACK=1 forces the pyhocon path (the gate uses it to prove the fallback is also iso).
"""
import os

from pyhocon import ConfigFactory
from pyhocon.config_tree import ConfigTree  # noqa: F401  (re-export)
from pyhocon.converter import HOCONConverter  # noqa: F401  (re-export)
from pyhocon.exceptions import (  # noqa: F401  (re-export)
    ConfigException,
    ConfigMissingException,
    ConfigSubstitutionException,
    ConfigWrongTypeException,
)

if not os.environ.get("PULSE_FORCE_FALLBACK"):
    try:  # pragma: no cover - depends on the native build
        from ._native import parse as _native_parse
        BACKEND = "rust"
    except ImportError:  # pragma: no cover
        _native_parse = None
        BACKEND = "python"
else:  # pragma: no cover
    _native_parse = None
    BACKEND = "python"


def parse_string(s):
    """Drop-in for `ConfigFactory.parse_string(s)` → `ConfigTree`."""
    if _native_parse is not None:
        try:
            return ConfigFactory.from_dict(_native_parse(s))
        except (NotImplementedError, ValueError):
            # out of scope / resolution failure / malformed input → pyhocon decides (resolves, or
            # raises ITS exact exception type: ConfigException/ParseException…). Iso guaranteed.
            pass
    return ConfigFactory.parse_string(s)


def parse_file(filename, encoding="utf-8", required=True, resolve=True):
    """Drop-in for `ConfigFactory.parse_file` → `ConfigTree` (includes resolved relative to the file)."""
    # resolve=False (unresolved tree) or no native backend → let pyhocon handle it
    if _native_parse is None or not resolve:
        return ConfigFactory.parse_file(filename, encoding=encoding, required=required, resolve=resolve)
    try:
        with open(filename, encoding=encoding) as f:
            content = f.read()
    except IOError:
        # missing/unreadable file: pyhocon applies the required/optional semantics
        return ConfigFactory.parse_file(filename, encoding=encoding, required=required, resolve=resolve)
    try:
        base = os.path.dirname(os.path.abspath(filename))
        return ConfigFactory.from_dict(_native_parse(content, base))
    except (NotImplementedError, ValueError):
        return ConfigFactory.parse_file(filename, encoding=encoding, required=required, resolve=resolve)


def parse_URL(url, *args, **kwargs):
    """Passthrough to pyhocon (network I/O — out of perf scope, a false friend)."""
    return ConfigFactory.parse_URL(url, *args, **kwargs)


def from_dict(dictionary, root=False):
    """Passthrough to `ConfigFactory.from_dict` (not a parse)."""
    return ConfigFactory.from_dict(dictionary, root=root)


# Historical alias: `pulse_pyhocon.parse` == `parse_string`.
parse = parse_string

__all__ = [
    "parse", "parse_string", "parse_file", "parse_URL", "from_dict",
    "ConfigFactory", "ConfigTree", "HOCONConverter",
    "ConfigException", "ConfigMissingException", "ConfigSubstitutionException",
    "ConfigWrongTypeException", "BACKEND",
]
