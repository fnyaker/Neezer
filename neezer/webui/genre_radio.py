"""Genre stations: a continuous radio anchored on one genre of the tree.

The routes register themselves on the shared ``webapi`` blueprint (imported at
the bottom of ``neezer/webui/__init__.py``), so they answer under ``/api`` with
the same session cookie and same-origin guard as every other route.

Contract (implemented by the genre-station lot):

* ``GET  /api/radio/genre/<style>``  -> ``{style, name, tracks}``
* ``POST /api/radio/genre/next``     -> ``{style, name, tracks}``

A station reads the person's taste restricted to the style and its subgenres
(``discover.style_taste``) and is assembled by the same radio/Flow machinery
(``discover.flow``), so it stays on the genre while still varying.
"""
