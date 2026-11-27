# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Réglages → Serveur : pochettes rangées dans le store d'archive.

The work is ``neezer/deezer/coverstore.py`` (a resumable library job on a
worker thread); this is its progress and the button that redoes a pass. Admin
only: it writes files. The first pass starts by itself on an upgraded server,
so nobody has to press anything for rule 7 to hold.
"""

from flask import current_app, jsonify, request

from . import admin_required, login_required, webapi


@webapi.route("/archive/covers", methods=["POST"])
@login_required
@admin_required
def archive_covers_start():
    from ..deezer import coverstore

    data = request.get_json(silent=True) or {}
    app = current_app._get_current_object()
    started = coverstore.start(app, force=bool(data.get("force")))
    return jsonify({"ok": True, "started": started, **coverstore.status(app.deezer)})


@webapi.route("/archive/covers")
@login_required
@admin_required
def archive_covers_status():
    from ..deezer import coverstore

    return jsonify(coverstore.status(current_app.deezer))
