# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Copyright (C) 2013-2025 Alban 'spl0k' Féron
#
# Distributed under terms of the GNU AGPLv3 license.

"""Subsonic playlists, with the collaborative read matrix applied on the wire.

A member of any role reads (the plan's v1 rule: a Subsonic client has no place
to send a version, so it never writes a collaborative playlist); the owner or an
instance admin writes. A non-member keeps the historical **error 50**. Every
count, duration and ``entry`` is the caller's VISIBLE view: somebody else's
private upload is simply absent, never listed as unavailable.
"""

import uuid

from flask import current_app, request
from peewee import JOIN, fn

from .. import playlist_access as placcess
from ..db import (
    Album,
    Artist,
    Folder,
    Playlist,
    PlaylistMember,
    PlaylistTrack,
    User,
    Track,
    db,
)

from . import get_entity, api_routing
from .exceptions import Forbidden, MissingParameter, NotFound


#: Ids per ``IN (...)``: SQLite's historical 999 host-parameter cap.
IN_CHUNK = 400


def _archive_added(tracks):
    """Adding a track to a playlist archives it, exactly as in the web app.

    Fail-soft — the playlist edit has already succeeded.
    """
    if not tracks:
        return
    from ..deezer import backfill

    try:
        backfill.archive_tracks(
            current_app._get_current_object(), tracks, event="on_playlist_add"
        )
    except Exception:
        pass


def _visible_counts(playlists, user):
    """``{playlist_id: (count, duration)}`` of the caller's visible rows.

    One grouped statement for the whole listing, whichever the playlist count:
    counting per playlist is the N+1 that ``as_subsonic_playlist`` already pays,
    and this must not double it.
    """
    ids = [playlist.id for playlist in playlists]
    if not ids:
        return {}
    query = (
        PlaylistTrack.select(
            PlaylistTrack.playlist,
            fn.COUNT(Track.id),
            fn.SUM(Track.duration),
        )
        .join(Track, on=(PlaylistTrack.track == Track.id))
        .where(PlaylistTrack.playlist.in_(ids))
        .group_by(PlaylistTrack.playlist)
        .tuples()
    )
    clause = Track.visible_clause(user)
    if clause is not None:
        query = query.where(clause)
    return {pid: (count or 0, total or 0) for pid, count, total in query}


def _subsonic_playlist(playlist, user, counts):
    """:meth:`Playlist.as_subsonic_playlist` with the caller's visible count."""
    info = playlist.as_subsonic_playlist(user)
    count, duration = counts.get(playlist.id, (0, 0))
    info["songCount"] = count
    info["duration"] = duration
    return info


def _visible_tracks(playlist, user):
    """The playlist's Track rows the caller may see, in stored order.

    One joined read for the whole page: each row comes with its album, its
    artist and both folders, so ``as_subsonic_child`` no longer pays four
    lookups per title. A 4 000-title playlist went from 28 016 statements to a
    handful; the visible filter is still applied per row, since a collaborator's
    private upload is listed but not readable.
    """
    root_folder = Folder.alias()
    query = (
        Track.select(Track, Album, Artist, Folder, root_folder)
        .join(Album, on=(Track.album == Album.id))
        # switch() back to Track before each join: Peewee's implicit join
        # context otherwise hangs the next model off the previous one, and the
        # relation it is not attached to is then re-read per row.
        .switch(Track)
        .join(Artist, on=(Track.artist == Artist.id))
        .switch(Track)
        .join(Folder, on=(Track.folder == Folder.id))
        .switch(Track)
        .join(
            root_folder, JOIN.LEFT_OUTER, on=(Track.root_folder == root_folder.id)
        )
        .switch(Track)
        .join(PlaylistTrack, on=(PlaylistTrack.track == Track.id))
        .where(PlaylistTrack.playlist == playlist.id)
        .order_by(PlaylistTrack.index)
    )
    return [track for track in query if track.readable_by(user)]


def _tracks_for_write(sids, user):
    """The rows a write may reference, in the order asked, in a few statements.

    Resolving them one by one was a SELECT per title; a malformed id is the
    Subsonic "invalid id" (error 0) it always was, and an unknown row or
    somebody else's private upload is still "not found" (error 70).
    """
    wanted = [uuid.UUID(str(sid)) for sid in sids]
    found = {}
    for i in range(0, len(wanted), IN_CHUNK):
        for track in Track.select().where(Track.id.in_(wanted[i : i + IN_CHUNK])):
            found[track.id] = track
    out = []
    for tid in wanted:
        track = found.get(tid)
        if track is None or not track.readable_by(user):
            raise NotFound("Track")
        out.append(track)
    return out


def _add_songs(playlist, songs, user):
    tracks = _tracks_for_write(songs, user)
    if tracks:
        playlist.add_many(tracks)
    return tracks


def _visible_indexes(playlist, indexes, user):
    """Map the caller's visible positions to stored ``PlaylistTrack.index``.

    An owner with no hidden row gets the identity mapping, which is what every
    existing client already sends. With a collaborator's private upload inside
    the list, a client can only name what it sees; the hidden rows stay where
    they are and are never removed by an off-by-one.
    """
    wanted = [int(index) for index in indexes]
    if not wanted:
        return []
    stored = [
        index
        for index, owner in (
            PlaylistTrack.select(PlaylistTrack.index, Track.owner)
            .join(Track, on=(PlaylistTrack.track == Track.id))
            .where(PlaylistTrack.playlist == playlist.id)
            .order_by(PlaylistTrack.index)
            .tuples()
        )
        if owner is None or getattr(user, "admin", False) or owner == user.id
    ]
    return [stored[i] if 0 <= i < len(stored) else -1 for i in wanted]


@api_routing("/getPlaylists")
def list_playlists():
    visibility = request.user
    query = (
        Playlist.select(Playlist, User)
        .join(User)
        .where(
            (Playlist.user == request.user)
            | Playlist.public
            | Playlist.id.in_(
                PlaylistMember.select(PlaylistMember.playlist).where(
                    PlaylistMember.user == request.user
                )
            )
        )
        .order_by(Playlist.name)
    )

    username = request.values.get("username")
    if username:
        if not request.user.admin:
            raise Forbidden()

        # get rather than join in the following query to raise an exception if the
        # requested user doesn't exist
        user = User.get(name=username)
        visibility = user
        query = (
            Playlist.select(Playlist, User)
            .join(User)
            .where(Playlist.user == user)
            .order_by(Playlist.name)
        )

    playlists = list(query)
    counts = _visible_counts(playlists, visibility)
    return request.formatter(
        "playlists",
        {"playlist": [_subsonic_playlist(p, request.user, counts) for p in playlists]},
    )


@api_routing("/getPlaylist")
def show_playlist():
    res = get_entity(Playlist)
    if placcess.role_for(res, request.user) is None:
        raise Forbidden()

    tracks = _visible_tracks(res, request.user)
    Track.prime_credits(tracks, playlist=res)
    Track.prime_subsonic(tracks, request.user, playlist=res)
    duration = sum(t.duration or 0 for t in tracks)
    info = res.as_subsonic_playlist(
        request.user, counts=(len(tracks), duration)
    )
    info["songCount"] = len(tracks)
    info["duration"] = duration
    info["entry"] = [
        t.as_subsonic_child(request.user, request.client) for t in tracks
    ]

    pf = getattr(current_app, "deezer_prefetch", None)
    if pf is not None:
        count = int(current_app.config["DEEZER"].get("preload_count") or 2)
        pf.enqueue_many((t for t in tracks if t.deezer_id), count)

    return request.formatter("playlist", info)


@api_routing("/createPlaylist")
def create_playlist():
    playlist_id, name = map(request.values.get, ("playlistId", "name"))
    # songId actually doesn't seem to be required
    songs = request.values.getlist("songId")
    playlist_id = uuid.UUID(playlist_id) if playlist_id else None

    if playlist_id:
        playlist = Playlist[playlist_id]

        if not placcess.role_can_manage(placcess.role_for(playlist, request.user)):
            raise Forbidden()

        with db.atomic():
            playlist.clear()
            if name:
                playlist.name = name
            added = _add_songs(playlist, songs, request.user)
            playlist.save()
            placcess.bump_version(playlist, None, request.user)
    elif name:
        with db.atomic():
            playlist = Playlist.create(user=request.user, name=name)
            PlaylistMember.create(
                playlist=playlist,
                user=request.user,
                role="owner",
                added=playlist.created,
            )
            added = _add_songs(playlist, songs, request.user)
    else:
        raise MissingParameter("playlistId or name")

    _archive_added(added)
    return request.formatter.empty


@api_routing("/deletePlaylist")
def delete_playlist():
    res = get_entity(Playlist)
    if not placcess.role_can_manage(placcess.role_for(res, request.user)):
        raise Forbidden()

    with db.atomic():
        PlaylistTrack.delete().where(PlaylistTrack.playlist == res).execute()
        res.delete_instance()

    return request.formatter.empty


@api_routing("/updatePlaylist")
def update_playlist():
    res = get_entity(Playlist, "playlistId")
    if not placcess.role_can_manage(placcess.role_for(res, request.user)):
        raise Forbidden()

    playlist = res
    name, comment, public = map(request.values.get, ("name", "comment", "public"))
    to_add, to_remove = map(
        request.values.getlist, ("songIdToAdd", "songIndexToRemove")
    )

    if name:
        playlist.name = name
    if comment:
        playlist.comment = comment
    if public:
        playlist.public = public in (True, "True", "true", 1, "1")

    with db.atomic():
        added = _add_songs(playlist, to_add, request.user)
        remove = _visible_indexes(playlist, to_remove, request.user)
        if remove:
            playlist.remove_at_indexes(remove)
        playlist.save()
        placcess.bump_version(playlist, None, request.user)

    _archive_added(added)
    return request.formatter.empty
