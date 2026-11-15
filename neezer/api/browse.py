# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Copyright (C) 2013-2022 Alban 'spl0k' Féron
#
# Distributed under terms of the GNU AGPLv3 license.

import re
import string

from flask import current_app, request
from peewee import fn

from ..db import Folder, Artist, Album, Track, TrackArtist

from . import get_entity, get_root_folder, api_routing
from .exceptions import NotFound


@api_routing("/getMusicFolders")
def list_folders():
    return request.formatter(
        "musicFolders",
        {
            "musicFolder": [
                {"id": str(f.id), "name": f.name}
                for f in Folder.select().where(Folder.root).order_by(Folder.name)
            ]
        },
    )


def build_ignored_articles_pattern():
    articles = current_app.config["WEBAPP"]["index_ignored_prefixes"]
    if articles is None:
        return None

    articles = articles.split()
    if not articles:
        return None

    return r"^(" + r" |".join(re.escape(a) for a in articles) + r" )"


def ignored_articles_str():
    articles = current_app.config["WEBAPP"]["index_ignored_prefixes"]
    if articles is None:
        return ""

    return " ".join(articles.split())


def build_indexes(source):
    indexes = {}
    pattern = build_ignored_articles_pattern()
    for item in source:
        name = item.name
        if pattern:
            name = re.sub(pattern, "", name, flags=re.I)
        index = name[0].upper()
        if index in string.digits:
            index = "#"
        elif index not in string.ascii_letters:
            index = "?"

        if index not in indexes:
            indexes[index] = []

        indexes[index].append((item, name))

    return indexes


@api_routing("/getIndexes")
def list_indexes():
    musicFolderId = request.values.get("musicFolderId")
    ifModifiedSince = request.values.get("ifModifiedSince")
    if ifModifiedSince:
        ifModifiedSince = int(ifModifiedSince) / 1000

    if musicFolderId is None:
        folders = Folder.select().where(Folder.root)[:]
    else:
        folders = [get_root_folder(musicFolderId)]

    last_modif = max(f.last_scan for f in folders)
    if ifModifiedSince is not None and last_modif < ifModifiedSince:
        return request.formatter(
            "indexes",
            {
                "lastModified": last_modif * 1000,
                "ignoredArticles": ignored_articles_str(),
            },
        )

    # The XSD lies, we don't return artists but a directory structure
    artists = []
    children = []
    for f in folders:
        artists += f.children[:]
        children += list(Track.visible(f.tracks, request.user))

    indexes = build_indexes(artists)
    return request.formatter(
        "indexes",
        {
            "lastModified": last_modif * 1000,
            "ignoredArticles": ignored_articles_str(),
            "index": [
                {
                    "name": k,
                    "artist": [
                        a.as_subsonic_artist(request.user)
                        for a, _ in sorted(v, key=lambda t: t[1].lower())
                    ],
                }
                for k, v in sorted(indexes.items())
            ],
            "child": [
                c.as_subsonic_child(request.user, request.client)
                for c in sorted(children, key=lambda t: t.sort_key())
            ],
        },
    )


@api_routing("/getMusicDirectory")
def show_directory():
    res = get_entity(Folder)
    return request.formatter(
        "directory", res.as_subsonic_directory(request.user, request.client)
    )


@api_routing("/getGenres")
def list_genres():
    return request.formatter(
        "genres",
        {
            "genre": [
                {"value": genre, "songCount": sc, "albumCount": ac}
                for genre, sc, ac in Track.select(
                    Track.genre, fn.count("*"), fn.count(Track.album.distinct())
                )
                .where(Track.genre.is_null(False))
                .group_by(Track.genre)
                .tuples()
            ]
        },
    )


@api_routing("/getArtists")
def list_artists():
    mfid = request.values.get("musicFolderId")

    clause = Track.visible_clause(request.user)
    if clause is None:
        query = Artist.select()
        if mfid is not None:
            folder = get_root_folder(mfid)
            query = Artist.select().join(Track).where(Track.root_folder == folder)
    else:
        tracks = Track.select(Track.artist).where(clause)
        credits = TrackArtist.select(TrackArtist.artist).join(Track).where(clause)
        if mfid is not None:
            folder = get_root_folder(mfid)
            tracks = tracks.where(Track.root_folder == folder)
            credits = credits.where(Track.root_folder == folder)
        query = Artist.select().where(
            (Artist.id.in_(tracks)) | (Artist.id.in_(credits))
        )

    indexes = build_indexes(query)
    return request.formatter(
        "artists",
        {
            "ignoredArticles": ignored_articles_str(),
            "index": [
                {
                    "name": k,
                    "artist": [
                        a.as_subsonic_artist(request.user)
                        for a, _ in sorted(v, key=lambda t: t[1].lower())
                    ],
                }
                for k, v in sorted(indexes.items())
            ],
        },
    )


@api_routing("/getArtist")
def artist_info():
    res = get_entity(Artist)
    albums = sorted(
        {t.album for t in Track.visible(res.tracks, request.user)},
        key=lambda a: a.sort_key(),
    )
    if not albums and res.albums.count() > 0:
        raise NotFound("Artist")
    info = res.as_subsonic_artist(request.user)
    info["albumCount"] = len(albums)
    info["album"] = [a.as_subsonic_album(request.user) for a in albums]

    return request.formatter("artist", info)


@api_routing("/getAlbum")
def album_info():
    res = get_entity(Album)
    tracks = Track.prime_credits(
        sorted(Track.visible(res.tracks, request.user), key=lambda t: t.sort_key())
    )
    if not tracks and res.tracks.count() > 0:
        raise NotFound("Album")
    info = res.as_subsonic_album(request.user)
    info["song"] = [
        t.as_subsonic_child(request.user, request.client) for t in tracks
    ]

    pf = getattr(current_app, "deezer_prefetch", None)
    if pf is not None:
        count = int(current_app.config["DEEZER"].get("preload_count") or 2)
        pf.enqueue_many((t for t in tracks if t.deezer_id), count)

    return request.formatter("album", info)


@api_routing("/getSong")
def track_info():
    res = get_entity(Track)
    if not res.readable_by(request.user):
        raise NotFound("Track")
    return request.formatter(
        "song", res.as_subsonic_child(request.user, request.client)
    )
