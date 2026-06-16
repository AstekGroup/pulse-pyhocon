"""Fallback pur-Python du drop-in pyhocon.

Quand l'extension native n'est pas compilée, on délègue à pyhocon lui-même (la référence) :
trivialement iso-fonctionnel, correct mais sans le gain de perf. Renvoie un vrai `ConfigTree`
(comme `ConfigFactory.parse_string`) — surface d'API identique au chemin natif.
"""
from pyhocon import ConfigFactory


def parse(s):
    return ConfigFactory.parse_string(s)
