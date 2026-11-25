# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""One episode's transcript as a file a person can keep or read elsewhere.

Four formats, because they answer four different needs: plain text to read or
paste, SubRip and WebVTT to load in a player or a video editor, Markdown to drop
in a note. All of them carry the same timed lines the panel shows; nothing is
summarised and no provider is involved.

Timestamps come from the segments' milliseconds, never from a clock: an
episode transcribed over several sessions has a start/end pair per line and the
export is the concatenation of those, not an approximation.
"""

from __future__ import annotations

import re

FORMATS = ("txt", "srt", "vtt", "md")

MIME = {
    "txt": "text/plain",
    "srt": "application/x-subrip",
    "vtt": "text/vtt",
    "md": "text/markdown",
}

_SPACES = re.compile(r"\s+")


def _clean(text):
    """One line of speech.

    A cue separator is a blank line, so a segment's text must never contain one:
    a line break inside a title or a run of spaces from the engine would split
    an SRT block and make every following timestamp wrong. Collapsing all
    whitespace to single spaces keeps the words and removes the separator.
    """
    return _SPACES.sub(" ", str(text or "")).strip()


def _clock(milliseconds):
    total = max(0, int(milliseconds or 0))
    return "%02d:%02d:%02d" % (total // 3_600_000, (total // 60_000) % 60, (total // 1000) % 60)


def _stamp(milliseconds, comma):
    total = max(0, int(milliseconds or 0))
    return "%02d:%02d:%02d%s%03d" % (
        total // 3_600_000,
        (total // 60_000) % 60,
        (total // 1000) % 60,
        "," if comma else ".",
        total % 1000,
    )


def _date(value):
    if value is None:
        return ""
    try:
        return value.strftime("%d/%m/%Y")
    except (AttributeError, ValueError):
        return ""


def _segment(segment):
    """(start_ms, end_ms, text) from a row (attributes) or a sidecar dict."""
    if isinstance(segment, dict):
        return segment.get("start_ms") or 0, segment.get("end_ms") or 0, segment.get("text")
    return (
        getattr(segment, "start_ms", 0) or 0,
        getattr(segment, "end_ms", 0) or 0,
        getattr(segment, "text", ""),
    )


def render(fmt, episode, channel, segments):
    """The whole transcript as text; ``ValueError`` on an unknown format."""
    fmt = str(fmt or "").lower()
    if fmt not in FORMATS:
        raise ValueError("unknown transcript format: %r" % (fmt,))
    title = _clean(getattr(episode, "title", "")) or "Épisode"
    show = _clean(getattr(channel, "title", ""))
    date = _date(getattr(episode, "publish_date", None))
    lines = []
    for segment in segments:
        start, end, text = _segment(segment)
        text = _clean(text)
        if text:
            lines.append((start, end, text))

    if fmt == "txt":
        head = ["Titre : " + title]
        if show:
            head.append("Émission : " + show)
        if date:
            head.append("Date : " + date)
        body = ["[%s] %s" % (_clock(start), text) for start, _end, text in lines]
        return "\n".join(head + [""] + body) + "\n"

    if fmt == "srt":
        blocks = [
            "%d\n%s --> %s\n%s\n" % (index, _stamp(start, True), _stamp(end, True), text)
            for index, (start, end, text) in enumerate(lines, 1)
        ]
        return "\n".join(blocks)

    if fmt == "vtt":
        cues = [
            "%s --> %s\n%s\n" % (_stamp(start, False), _stamp(end, False), text)
            for start, end, text in lines
        ]
        return "WEBVTT\n\n" + "\n".join(cues)

    head = ["# " + title, ""]
    meta = show + (" — " + date if date else "")
    if meta.strip(" —"):
        head.append("*%s*" % meta)
        head.append("")
    body = ["**[%s]** %s" % (_clock(start), text) for start, _end, text in lines]
    return "\n".join(head) + "\n" + "\n\n".join(body) + "\n"
