# This file is part of Neezer.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Content-Type of the formats this server transcodes TO.

``mimetypes.guess_type`` answers from the interpreter's table plus the host's
``/etc/mime.types``, and a slim container image ships neither for ``.ogg``: an
Opus transcode was then announced as ``application/octet-stream``, and players
that trust the header refused the stream even though the bytes were audio. The
targets the transcoders write are a closed set, so they are listed here instead
of asking the host.

``neezer/db.py`` announces the same value as ``transcodedContentType`` and
should read this table too.
"""

import mimetypes

#: Container the transcoder writes (suffix without the dot) -> Content-Type.
TRANSCODE_MIMETYPES = {
    "aac": "audio/aac",
    "flac": "audio/flac",
    "m4a": "audio/mp4",
    "mp3": "audio/mpeg",
    "mp4": "audio/mp4",
    "oga": "audio/ogg",
    "ogg": "audio/ogg",
    "opus": "audio/ogg",
    "wav": "audio/wav",
    "wma": "audio/x-ms-wma",
}


def transcode_mimetype(suffix):
    """The Content-Type served for a transcode target (``suffix``, with or without dot)."""
    key = str(suffix or "").strip().lstrip(".").lower()
    known = TRANSCODE_MIMETYPES.get(key)
    if known:
        return known
    # An operator's own target that the table does not know: the host's guess
    # is still better than nothing.
    guess = mimetypes.guess_type("dummyname." + key, False)[0]
    return guess or "application/octet-stream"
