"""Fallback pur-Python du drop-in pyhocon.

Quand l'extension native n'est pas compilée, on délègue à pyhocon lui-même (la référence) :
trivialement iso-fonctionnel (c'est l'implémentation d'origine), correct mais SANS le gain de
perf — c'est le rôle d'un fallback (le wheel natif apporte la vitesse). On renvoie un dict plat,
comme le backend Rust, pour une API de sortie identique sur le périmètre couvert.
"""
from pyhocon import ConfigFactory
from pyhocon.config_tree import ConfigTree


def _to_plain(x):
    if isinstance(x, ConfigTree):
        return {k: _to_plain(v) for k, v in x.items()}
    if isinstance(x, list):
        return [_to_plain(v) for v in x]
    return x


def parse(s):
    return _to_plain(ConfigFactory.parse_string(s))
