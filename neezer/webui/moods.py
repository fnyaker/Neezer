"""Mood mixes built from what the server already measured.

A mood is a set of measured bands (energy, tempo, spectral colour) over the
library, never a hand-typed tag: ``neezer/moods.py`` holds the definitions and
the assembly, this module exposes them.

Contract (implemented by the mood-mix lot):

* ``GET /api/moods``            -> ``{moods: [{id, name, subtitle, tracks, ready, cover}]}``
* ``GET /api/moods/<id>``       -> ``{id, name, subtitle, tracks}``

The routes register themselves on the shared ``webapi`` blueprint, imported at
the bottom of ``neezer/webui/__init__.py``. No provider is asked: a mood is
made from archived files whose analysis the server already holds.
"""
