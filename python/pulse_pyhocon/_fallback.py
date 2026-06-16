"""Pure-Python fallback for the pyhocon drop-in.

When the native extension is not compiled, we delegate to pyhocon itself (the reference):
trivially iso-functional, correct but without the perf gain. Returns a real `ConfigTree`
(like `ConfigFactory.parse_string`) — same API surface as the native path.
"""
from pyhocon import ConfigFactory


def parse(s):
    return ConfigFactory.parse_string(s)
