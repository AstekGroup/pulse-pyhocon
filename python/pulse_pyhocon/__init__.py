"""Drop-in Rust/PyO3 de pyhocon (axe B) — API complète renvoyant un vrai `ConfigTree`.

`parse`/`parse_string`/`parse_file` renvoient un `pyhocon.ConfigTree` → toute l'API pyhocon marche
(`get`, `get_string`/`get_int`/`get_float`/`get_bool`/`get_list`/`get_config`, accès pointé, défaut,
`with_fallback`, `as_plain_ordered_dict`, `HOCONConverter.to_hocon/json/yaml/properties`…).

Chemin rapide : le noyau Rust parse vite (dict plat), qu'on enveloppe en `ConfigTree` via
`ConfigFactory.from_dict` (le constructeur dict→arbre canonique de pyhocon → iso, vérifié structure +
getters). On NE réimplémente PAS les getters/converters en Rust (ce n'est pas le chemin chaud) : on
réutilise ceux de pyhocon sur l'arbre. Tout construct hors chemin rapide ou échec de résolution →
`NotImplementedError` → délégation transparente à pyhocon (l'oracle). Iso garantie.

PULSE_FORCE_FALLBACK=1 force le chemin pyhocon (le gate l'utilise pour prouver l'iso du fallback).
"""
import os

from pyhocon import ConfigFactory
from pyhocon.config_tree import ConfigTree  # noqa: F401  (ré-export)
from pyhocon.converter import HOCONConverter  # noqa: F401  (ré-export)
from pyhocon.exceptions import (  # noqa: F401  (ré-export)
    ConfigException,
    ConfigMissingException,
    ConfigSubstitutionException,
    ConfigWrongTypeException,
)

if not os.environ.get("PULSE_FORCE_FALLBACK"):
    try:  # pragma: no cover - dépend du build natif
        from ._native import parse as _native_parse
        BACKEND = "rust"
    except ImportError:  # pragma: no cover
        _native_parse = None
        BACKEND = "python"
else:  # pragma: no cover
    _native_parse = None
    BACKEND = "python"


def parse_string(s):
    """Drop-in de `ConfigFactory.parse_string(s)` → `ConfigTree`."""
    if _native_parse is not None:
        try:
            return ConfigFactory.from_dict(_native_parse(s))
        except NotImplementedError:
            pass  # hors périmètre / échec de résolution → pyhocon (exact)
    return ConfigFactory.parse_string(s)


def parse_file(filename, encoding="utf-8", required=True, resolve=True):
    """Drop-in de `ConfigFactory.parse_file` → `ConfigTree` (includes résolus relativement au fichier)."""
    # resolve=False (arbre non résolu) ou pas de natif → pyhocon gère
    if _native_parse is None or not resolve:
        return ConfigFactory.parse_file(filename, encoding=encoding, required=required, resolve=resolve)
    try:
        with open(filename, encoding=encoding) as f:
            content = f.read()
    except IOError:
        # fichier manquant/illisible : pyhocon applique la sémantique required/optionnel
        return ConfigFactory.parse_file(filename, encoding=encoding, required=required, resolve=resolve)
    try:
        base = os.path.dirname(os.path.abspath(filename))
        return ConfigFactory.from_dict(_native_parse(content, base))
    except NotImplementedError:
        return ConfigFactory.parse_file(filename, encoding=encoding, required=required, resolve=resolve)


def parse_URL(url, *args, **kwargs):
    """Passthrough vers pyhocon (I/O réseau — hors périmètre perf, faux ami)."""
    return ConfigFactory.parse_URL(url, *args, **kwargs)


def from_dict(dictionary, root=False):
    """Passthrough vers `ConfigFactory.from_dict` (pas un parse)."""
    return ConfigFactory.from_dict(dictionary, root=root)


# Alias historique : `pulse_pyhocon.parse` == `parse_string`.
parse = parse_string

__all__ = [
    "parse", "parse_string", "parse_file", "parse_URL", "from_dict",
    "ConfigFactory", "ConfigTree", "HOCONConverter",
    "ConfigException", "ConfigMissingException", "ConfigSubstitutionException",
    "ConfigWrongTypeException", "BACKEND",
]
