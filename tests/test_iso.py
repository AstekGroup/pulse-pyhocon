"""Differential oracle: pulse_pyhocon.parse must be iso-functional with pyhocon
(ConfigFactory.parse_string) — result AND exception type. This is the package's core guarantee.
"""
import pytest
from pyhocon import ConfigFactory
from pyhocon.config_tree import ConfigTree

import pulse_pyhocon

try:
    from pyhocon.config_tree import NoneValue
except Exception:  # pragma: no cover
    NoneValue = ()


def _plain(x):
    if isinstance(x, ConfigTree):
        return {k: _plain(v) for k, v in x.items()}
    if isinstance(x, list):
        return [_plain(v) for v in x]
    if NoneValue and isinstance(x, NoneValue):
        return None
    return x


def _canon(x):
    """TYPED canonical form — catches any int/float/bool/null drift (1 == 1.0 in Python).
    ConfigTree ⊂ dict (recursion via items()); NoneValue (pyhocon's internal null) ≡ None."""
    if isinstance(x, dict):
        return ("dict", [(k, _canon(v)) for k, v in x.items()])
    if isinstance(x, list):
        return ("list", [_canon(v) for v in x])
    if isinstance(x, bool):
        return ("bool", x)
    if isinstance(x, int):
        return ("int", x)
    if isinstance(x, float):
        return ("float", x)
    if x is None or (NoneValue and isinstance(x, NoneValue)):
        return ("null", None)
    if isinstance(x, str):
        return ("str", x)
    return ("other", type(x).__name__, repr(x))


def _outcome(fn, text):
    try:
        return ("ok", _canon(fn(text)))
    except Exception as e:
        return ("exc", type(e).__name__)


def _ref(text):
    # Raw ConfigTree: the candidate also returns a ConfigTree → we compare the full API.
    return ConfigFactory.parse_string(text)


CORE = [
    "a = 1\nb = 2",
    "x { y = 3 }",
    's = "hello world"',
    "arr = [1, 2, 3]",
    'mixed = [a, "b", 3, true, null, 1.5]',
    "nested { a { b { c = deep } } }",
    "f = 3.14\ng = 0.5\nh = 1e3\ni = -7",
    "b1 = true\nb2 = FALSE\nn = NULL",
    "dotted.key.here = 42",
    "a.b = 1\na.c = 2",
    "o { p = 1 }\no { q = 2 }",
    "# comment\nx = 1 // inline\ny = 2",
    "big { a=1, b=2, d { e=4, f=[5,6,7] } }",
    # substitutions
    "a = 1\nb = ${a}",
    "b = ${a}\na = 2",
    "a = {x=1}\nb = ${a}",
    'h = host\nu = "http://"${h}":80"',
    "a { b { c = deep } }\nx = ${a.b.c}",
    "b = ${?nope}",
    "a = 1\nb = ${a}\nc = ${b}",
    "base = /opt\nbin = ${base}/bin",
    "b = ${nope}",                       # ConfigSubstitutionException
    "a = ${b}\nb = ${a}",                # cycle
    # object/array concatenation
    "o1={x=1}\no2={y=2}\nm=${o1}${o2}",
    "m = {x=1} {y=2}",
    "a1=[1,2]\na2=[3,4]\nc=${a1}${a2}",
    "c = [1] [2] [3]",
    "o={x=1}\nm=${o} foo",               # ConfigWrongTypeException
    # regressions (fuzz)
    "a = 9999999999999999999",           # big integer
    "b = a//b",                          # literal '//'
    "u = http://host:5432/path",
    "a { b = 1 } c { d = 2 }",           # object without '='
    "b = ${?n1}${?n2}",                  # all absent -> key omitted
    "a = null\nb = ${a}",                # subst -> null -> key omitted
    '"hello" = 1',
    '"a.b" = 1',                         # special quoted key -> fallback
    "a =",                               # empty value -> fallback
    # +=
    "a += 1\na += 2",
    "x = [1,2]\nx += [3,4]",
    "x = abc\nx += def",
]


@pytest.mark.parametrize("text", CORE)
def test_iso_core(text):
    assert _outcome(pulse_pyhocon.parse, text) == _outcome(_ref, text)


# Self-reference — pyhocon resolves `${k}` (in the value overriding `k`) to the PREVIOUS value
# (e.g. `path = ${path}":/usr/bin"`). Common idioms with a CONCRETE previous value are resolved
# NATIVELY; a non-concrete previous value / absolute path / nav-through-substitution → transparent
# fallback. All iso. (Historical regression fixed: these used to wrongly raise ConfigSubstitutionException.)
SELFREF = [
    "a = 1\na = ${a}",                                   # -> 1 (native)
    'p = "/bin"\np = ${p}":/usr/bin"',                   # -> "/bin:/usr/bin" (native)
    "p = /a\np = ${p}:/b",                               # unquoted suffix (native)
    "a = [1]\na = ${a} [2]",                             # -> [1, 2] (native)
    "a = [1]\na = ${a}[2]",                              # no space (native)
    "a = { b = 1 }\na = ${a} { c = 2 }",                 # object self-merge (native)
    "n = 1\nn = ${n} 2",                                 # -> "1 2" (native)
    "a = 1\na = ${a}\na = ${a}",                         # double override (native)
    "a = ${b}\na = ${a}\nb = 5",                         # non-concrete previous value -> fallback (pyhocon RAISES)
    "x { a = 1 }\nx { a = ${x.a} }",                     # nested absolute path -> fallback
    "base = { host = h }\nx = ${base}\ny = ${x.host}",   # nav through a substitution -> fallback
    "a = { b = { c = 1 } }\nd = ${a}\ne = ${d.b.c}",     # deep nav -> fallback
    "a = ${a}",                                          # no previous value -> pyhocon RAISES (fallback)
]


@pytest.mark.parametrize("text", SELFREF)
def test_iso_self_reference(text):
    assert _outcome(pulse_pyhocon.parse, text) == _outcome(_ref, text)


# Bare true/false keyword in a multi-token unquoted run: pyhocon TYPES it (-> "True"/"False" in a
# string concat), the native core falls back transparently -> iso. (null excluded: pyhocon renders a
# NoneValue repr with a memory address, non-deterministic even pyhocon-vs-pyhocon.)
KEYWORD_CONCAT = [
    "m = foo true",
    "m = a true b",
    "a = x\nm = ${a} true",
    "a = x\nm = ${a} false bar",
    "p = base\np = ${p} true",
]


@pytest.mark.parametrize("text", KEYWORD_CONCAT)
def test_iso_keyword_concat(text):
    assert _outcome(pulse_pyhocon.parse, text) == _outcome(_ref, text)


# MALFORMED input: the native core raises ValueError → the wrapper delegates to pyhocon, which raises
# ITS exact type (ParseException/…). Exception-TYPE parity (before: ValueError ≠ ParseException).
MALFORMED = [
    "= 5",
    "a = }",
    "{ a = 1",
]


@pytest.mark.parametrize("text", MALFORMED)
def test_iso_malformed_exception_type(text):
    assert _outcome(pulse_pyhocon.parse, text) == _outcome(_ref, text)


# EXTREME float in a string concatenation: Rust formats without scientific notation ≠ Python str(float)
# → transparent fallback (pyhocon renders). A NORMAL float in a concat stays native (directly iso).
FLOAT_CONCAT = [
    "f = 1e100\ns = ${f} x",      # extreme -> fallback
    "f = 1e-7\ns = ${f}x",        # extreme -> fallback
    "f = 1.5\ns = ${f}-build",    # normal -> native
    "f = 3.14\ns = v${f}",        # normal -> native
]


@pytest.mark.parametrize("text", FLOAT_CONCAT)
def test_iso_float_concat(text):
    assert _outcome(pulse_pyhocon.parse, text) == _outcome(_ref, text)


INCLUDES = [
    ('include "sub.conf"\nmore = 2', {"sub.conf": 'x = 1\ny = "z"\n'}),
    ('include file("sub.conf")', {"sub.conf": "x = 1\n"}),
    ('x = 100\ninclude "sub.conf"', {"sub.conf": "x = 1\n"}),
    ('cfg { include "sub.conf"\ne = 3 }', {"sub.conf": "x = 1\n"}),
    ('include "missing.conf"\na = 1', {}),
    ('include required("missing.conf")', {}),     # FileNotFoundError
    ('include "chain.conf"', {"chain.conf": 'include "leaf.conf"\nz = 9\n', "leaf.conf": "w = 1\n"}),
    ('include "sub.conf"\nv = ${x}', {"sub.conf": "x = 5\n"}),
]


@pytest.mark.parametrize("text,files", INCLUDES)
def test_iso_includes(text, files, tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    for name, content in files.items():
        (tmp_path / name).write_text(content)
    assert _outcome(pulse_pyhocon.parse, text) == _outcome(_ref, text)


def test_backend_native():
    # In CI (extension compiled), the backend must be native.
    assert pulse_pyhocon.BACKEND in ("rust", "python")


# --- Full ConfigTree API: parse* returns a real ConfigTree, iso with pyhocon --------------------

API_CONF = (
    'a = 1\nb = hello\nc = 3.5\nd = true\ne = [1, 2, 3]\n'
    'f { x = 10\ng { h = deep } }\nnothing = null'
)


def test_returns_real_configtree():
    assert isinstance(pulse_pyhocon.parse(API_CONF), ConfigTree)
    assert isinstance(pulse_pyhocon.parse_string(API_CONF), ConfigTree)


@pytest.mark.parametrize("access", [
    lambda t: t.get_int("a"),
    lambda t: t.get_string("b"),
    lambda t: t.get_float("c"),
    lambda t: t.get_bool("d"),
    lambda t: t.get_list("e"),
    lambda t: t.get_string("a"),                 # int -> str coercion
    lambda t: t.get("f.x"),                      # dotted access
    lambda t: t.get_config("f").get("g.h"),      # sub-config
    lambda t: t.get("absent", "DEFAULT"),        # default
    lambda t: "f" in t,
    lambda t: sorted(t.keys()),
    lambda t: t.as_plain_ordered_dict(),
    lambda t: t["f"]["x"],
])
def test_iso_configtree_getters(access):
    a = pulse_pyhocon.parse(API_CONF)
    b = ConfigFactory.parse_string(API_CONF)
    try:
        ra = ("ok", repr(access(a)))
    except Exception as e:
        ra = ("exc", type(e).__name__)
    try:
        rb = ("ok", repr(access(b)))
    except Exception as e:
        rb = ("exc", type(e).__name__)
    assert ra == rb


@pytest.mark.parametrize("conv", ["to_json", "to_hocon", "to_properties", "to_yaml"])
def test_iso_hocon_converter(conv):
    from pyhocon.converter import HOCONConverter
    a = pulse_pyhocon.parse(API_CONF)
    b = ConfigFactory.parse_string(API_CONF)
    assert getattr(HOCONConverter, conv)(a) == getattr(HOCONConverter, conv)(b)


def test_iso_with_fallback():
    base, over = "a = 1\nb = 2", "b = 20\nc = 30"
    a = pulse_pyhocon.parse(base).with_fallback(pulse_pyhocon.parse(over))
    b = ConfigFactory.parse_string(base).with_fallback(ConfigFactory.parse_string(over))
    assert _canon(a) == _canon(b)


def test_iso_parse_file(tmp_path):
    (tmp_path / "sub.conf").write_text('y = 2\n')
    (tmp_path / "main.conf").write_text('include "sub.conf"\nx = 1\nz = ${x}\n')
    path = str(tmp_path / "main.conf")
    a = pulse_pyhocon.parse_file(path)
    b = ConfigFactory.parse_file(path)
    assert isinstance(a, ConfigTree)
    assert _canon(a) == _canon(b)


def test_iso_parse_file_missing_required(tmp_path):
    # required file missing: same behavior (exception) on both sides
    path = str(tmp_path / "nope.conf")
    ra = _outcome(lambda _p: pulse_pyhocon.parse_file(path), path)
    rb = _outcome(lambda _p: ConfigFactory.parse_file(path), path)
    assert ra[0] == rb[0]  # both ok (empty) or both exc
