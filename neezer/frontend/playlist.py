# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Copyright (C) 2013-2025 Alban 'spl0k' Féron
#
# Distributed under terms of the GNU AGPLv3 license.

import re
import uuid

from flask import Response, flash, redirect, render_template, request, url_for
from functools import wraps
from urllib.parse import quote

from .. import playlist_access as placcess
from ..db import Playlist, PlaylistTrack, db

from . import frontend


@frontend.route("/playlist")
def playlist_index():
    return render_template(
        "playlists.html",
        mine=Playlist.select().where(Playlist.user == request.user),
        others=Playlist.select().where(Playlist.user != request.user, Playlist.public),
    )


def resolve_and_inject_playlist(func):
    @wraps(func)
    def decorated(uid):
        try:
            uid = uuid.UUID(uid)
        except ValueError:
            flash("Invalid playlist id", "warning")
            return redirect(url_for("frontend.playlist_index"))

        try:
            playlist = Playlist[uid]
        except Playlist.DoesNotExist:
            flash("Unknown playlist", "warning")
            return redirect(url_for("frontend.playlist_index"))

        if not placcess.can_read(playlist, request.user):
            flash("Unknown playlist", "warning")
            return redirect(url_for("frontend.playlist_index"))

        return func(uid, playlist)

    return decorated


class _VisiblePlaylist:
    """A playlist as one person sees it: private uploads of others are absent."""

    def __init__(self, playlist, user):
        self._playlist = playlist
        self._user = user

    def __getattr__(self, name):
        return getattr(self._playlist, name)

    def get_tracks(self):
        return [
            track
            for track in self._playlist.get_tracks()
            if track.readable_by(self._user)
        ]


@frontend.route("/playlist/<uid>")
@resolve_and_inject_playlist
def playlist_details(uid, playlist):
    return render_template(
        "playlist.html", playlist=_VisiblePlaylist(playlist, request.user)
    )


@frontend.route("/playlist/<uid>/export")
@resolve_and_inject_playlist
def playlist_export(uid, playlist):
    return Response(
        render_template(  # nosemgrep: a plain-text playlist, not an HTML page
            "playlist_export.m3u",
            playlist=_VisiblePlaylist(playlist, request.user),
        ),
        mimetype="audio/mpegurl",
        # The name is the user's: a quote, a semicolon or a newline in it must
        # not reach the header raw. RFC 6266 form, with an ASCII fallback.
        headers={
            "Content-disposition": "attachment; filename=\"{}.m3u\"; "
            "filename*=UTF-8''{}.m3u".format(
                re.sub(r'[^A-Za-z0-9 ._-]', "_", playlist.name) or "playlist",
                quote(playlist.name, safe=""),
            )
        },
    )


@frontend.route("/playlist/<uid>", methods=["POST"])
@resolve_and_inject_playlist
def playlist_update(uid, playlist):
    if not placcess.can_manage(playlist, request.user):
        flash("You're not allowed to edit this playlist", "danger")
    elif not request.form.get("name"):
        flash("Missing playlist name", "danger")
    else:
        playlist.name = request.form.get("name")
        playlist.public = request.form.get("public") in (
            True,
            "True",
            1,
            "1",
            "on",
            "checked",
        )
        with db.atomic():
            playlist.save()
            placcess.bump_version(playlist, None, request.user)
        flash("Playlist updated.", "success")

    return playlist_details(str(uid))


@frontend.route("/playlist/del/<uid>", methods=["POST"])
@resolve_and_inject_playlist
def playlist_delete(uid, playlist):
    if not placcess.can_manage(playlist, request.user):
        flash("You're not allowed to delete this playlist", "danger")
    else:
        with db.atomic():
            PlaylistTrack.delete().where(PlaylistTrack.playlist == playlist).execute()
            playlist.delete_instance()
        flash("Playlist deleted", "success")

    return redirect(url_for("frontend.playlist_index"))
