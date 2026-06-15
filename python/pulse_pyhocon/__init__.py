"""Drop-in Rust/PyO3 de `pyhocon.ConfigFactory.parse_string` (axe B).

Chemin rapide : noyau Rust (parsing + substitutions + includes + concat objets/arrays). Pour
les constructs délibérément non répliqués (ex. `+=`, dont l'implémentation pyhocon 0.3.63 est
buggée), le Rust lève `NotImplementedError` et l'on **délègue de façon transparente à pyhocon**
(fallback) — le drop-in reste donc 100 % iso, sans répliquer le bug. Si l'extension native est
absente, tout passe par le fallback (= pyhocon).

PULSE_FORCE_FALLBACK=1 force le chemin pyhocon (le gate l'utilise pour prouver l'iso du fallback).
"""
import os

from ._fallback import parse as _fallback_parse

if not os.environ.get("PULSE_FORCE_FALLBACK"):
    try:  # pragma: no cover - dépend du build natif
        from ._native import parse as _native_parse  # type: ignore

        def parse(s):
            try:
                return _native_parse(s)
            except NotImplementedError:
                # construct hors périmètre du chemin rapide → pyhocon (exact)
                return _fallback_parse(s)

        BACKEND = "rust"
    except ImportError:  # pragma: no cover
        parse = _fallback_parse
        BACKEND = "python"
else:  # pragma: no cover
    parse = _fallback_parse
    BACKEND = "python"

__all__ = ["parse", "BACKEND"]
