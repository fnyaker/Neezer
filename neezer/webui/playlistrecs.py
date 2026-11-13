"""Recommendations for the bottom of a playlist.

The playlist itself is the seed: its titles' stored sound neighbours, their
artists and their styles feed the existing radio assembler, minus what the
playlist already holds. Library-only, no provider call.

Contract (implemented by the playlist-recommendations lot):

* ``GET /api/playlist/<playlist_id>/recommendations?limit=12``
  -> ``{playlist, tracks}``

The routes register themselves on the shared ``webapi`` blueprint, imported at
the bottom of ``neezer/webui/__init__.py``.
"""
