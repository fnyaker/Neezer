"""A person's own listening history, read back.

The rows already exist in ``listen_event`` (``neezer/listening.py`` records
every track change with how long it played and whether it was a listen or a
skip). This module only READS them, for the History screen.

Contract (implemented by the history lot):

* ``GET /api/history?before=&before_id=&limit=&outcome=``
  -> ``{events, next, summary}``

Only the signed-in person's rows are served; an admin has no view of somebody
else's history. The routes register themselves on the shared ``webapi``
blueprint, imported at the bottom of ``neezer/webui/__init__.py``.
"""
