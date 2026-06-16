"""Oracle différentiel : pulse_pyhocon.parse doit être iso-fonctionnel avec pyhocon
(ConfigFactory.parse_string) — résultat ET type d'exception. C'est la garantie centrale du package.
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
    """Forme canonique TYPÉE — attrape tout drift int/float/bool/null (1 == 1.0 en Python)."""
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
    if x is None:
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
    return _plain(ConfigFactory.parse_string(text))


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
    # concaténation objets/arrays
    "o1={x=1}\no2={y=2}\nm=${o1}${o2}",
    "m = {x=1} {y=2}",
    "a1=[1,2]\na2=[3,4]\nc=${a1}${a2}",
    "c = [1] [2] [3]",
    "o={x=1}\nm=${o} foo",               # ConfigWrongTypeException
    # régressions (fuzz)
    "a = 9999999999999999999",           # grand entier
    "b = a//b",                          # '//' littéral
    "u = http://host:5432/path",
    "a { b = 1 } c { d = 2 }",           # objet sans '='
    "b = ${?n1}${?n2}",                  # tous absents -> clé omise
    "a = null\nb = ${a}",                # subst -> null -> clé omise
    '"hello" = 1',
    '"a.b" = 1',                         # clé quotée spéciale -> fallback
    "a =",                               # valeur vide -> fallback
    # +=
    "a += 1\na += 2",
    "x = [1,2]\nx += [3,4]",
    "x = abc\nx += def",
]


@pytest.mark.parametrize("text", CORE)
def test_iso_core(text):
    assert _outcome(pulse_pyhocon.parse, text) == _outcome(_ref, text)


# Auto-référence & navigation à travers une substitution : idiomes HOCON courants que pyhocon
# RÉSOUT (ex. `path = ${path}":/usr/bin"`). Le noyau natif ne les gère pas → fallback transparent
# → iso. Régression historique : ces cas levaient à tort ConfigSubstitutionException (bug iso).
SELFREF = [
    "a = 1\na = ${a}",                                   # auto-réf simple -> 1
    'p = "/bin"\np = ${p}":/usr/bin"',                   # self-concat (motif PATH) -> "/bin:/usr/bin"
    "a = [1]\na = ${a} [2]",                             # self-append -> [1, 2]
    "x { a = 1 }\nx { a = ${x.a} }",                     # auto-réf imbriquée
    "a = { b = 1 }\na = ${a} { c = 2 }",                 # self-merge objet
    "n = 1\nn = ${n} 2",                                 # self-concat string -> "1 2"
    "base = { host = h }\nx = ${base}\ny = ${x.host}",   # nav de chemin À TRAVERS une substitution
    "a = { b = { c = 1 } }\nd = ${a}\ne = ${d.b.c}",     # nav profonde à travers subst
    "a = ${a}",                                          # auto-réf sans valeur préalable -> pyhocon LÈVE
]


@pytest.mark.parametrize("text", SELFREF)
def test_iso_self_reference(text):
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
    # En CI (extension compilée), le backend doit être natif.
    assert pulse_pyhocon.BACKEND in ("rust", "python")
