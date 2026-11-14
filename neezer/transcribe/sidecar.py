# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The transcript on the data disk, next to the episode it belongs to.

``<episode>.transcript.json`` is the canonical text: format version, language,
engine and model, duration, and the timed segments. ``<episode>.transcript.srt``
is the same text as subtitles, regenerated from the JSON every time. Both are
written to a temporary file in the same directory and renamed, so a crash or a
full disk leaves the previous copy intact rather than half a JSON file.

The database is an INDEX of these files, never their owner: a missing sidecar
does not destroy a row, a missing row does not destroy a sidecar. No path here
comes from a client; the audio path is the episode's own archive path and its
real path must stay under ``archive_dir/Podcasts``. Nothing in this module
imports the database or Flask.
"""

from __future__ import annotations

import json
import os
import tempfile

FORMAT_VERSION = 1
MAX_SEGMENTS = 100_000
MAX_TEXT = 2000
MAX_SIDECAR_BYTES = 8 * 1024 * 1024
PODCAST_DIR = "Podcasts"


class SidecarError(ValueError):
    pass


def json_path(audio_path):
    base, _extension = os.path.splitext(str(audio_path))
    return base + ".transcript.json"


def srt_path(audio_path):
    base, _extension = os.path.splitext(str(audio_path))
    return base + ".transcript.srt"


def root_for(archive_dir):
    if not archive_dir:
        return None
    try:
        return os.path.realpath(os.path.join(str(archive_dir), PODCAST_DIR))
    except (OSError, ValueError):
        return None


def inside(path, root):
    if not path or not root:
        return False
    try:
        real = os.path.realpath(path)
        return os.path.commonpath([real, root]) == root
    except (OSError, ValueError):
        return False


def safe_audio(audio_path, root):
    if not audio_path or not root:
        raise SidecarError("archive podcast introuvable")
    if not os.path.isfile(audio_path):
        raise SidecarError("l'audio archivé de l'épisode est absent")
    real = os.path.realpath(audio_path)
    if not inside(real, root):
        raise SidecarError("l'audio de l'épisode est hors de l'archive Podcasts")
    return real


def _clean_text(raw):
    if not isinstance(raw, str):
        raise SidecarError("un segment n'a pas de texte")
    text = "".join(
        " " if ord(char) < 0x20 or ord(char) == 0x7F else char for char in raw
    )
    text = " ".join(text.split())
    if not text or len(text) > MAX_TEXT:
        if not text:
            raise SidecarError("un segment a un texte vide")
        text = text[:MAX_TEXT].strip()
    return text


def _ms(value, what):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise SidecarError(f"{what} n'est pas un nombre")
    milliseconds = int(round(value))
    if milliseconds < 0:
        raise SidecarError(f"{what} est négatif")
    return milliseconds


def normalize(payload):
    """Validate a sidecar document and return it with clean segments."""
    if not isinstance(payload, dict):
        raise SidecarError("le sidecar n'est pas un objet JSON")
    version = payload.get("format")
    if version != FORMAT_VERSION:
        raise SidecarError("version de sidecar inconnue")
    segments = payload.get("segments")
    if not isinstance(segments, list):
        raise SidecarError("le sidecar n'a pas de segments")
    if len(segments) > MAX_SEGMENTS:
        raise SidecarError("le sidecar a trop de segments")
    clean = []
    for item in segments:
        if not isinstance(item, dict):
            raise SidecarError("un segment n'est pas un objet")
        start = _ms(item.get("start_ms"), "start_ms")
        end = _ms(item.get("end_ms"), "end_ms")
        if end <= start:
            raise SidecarError("un segment se termine avant de commencer")
        clean.append(
            {"start_ms": start, "end_ms": end, "text": _clean_text(item.get("text"))}
        )
    clean.sort(key=lambda segment: (segment["start_ms"], segment["end_ms"]))
    duration = _ms(payload.get("duration_ms") or 0, "duration_ms")
    result = {
        "format": FORMAT_VERSION,
        "episode_id": str(payload.get("episode_id") or ""),
        "language": payload.get("language"),
        "engine": payload.get("engine"),
        "model": payload.get("model"),
        "engine_version": payload.get("engine_version"),
        "duration_ms": duration,
        "created": payload.get("created"),
        "updated": payload.get("updated"),
        "segments": clean,
    }
    return result


def build(episode_id, language, engine, model, engine_version, duration_ms,
          segments, created, updated):
    return normalize(
        {
            "format": FORMAT_VERSION,
            "episode_id": str(episode_id),
            "language": language,
            "engine": engine,
            "model": model,
            "engine_version": engine_version,
            "duration_ms": int(duration_ms or 0),
            "created": created,
            "updated": updated,
            "segments": list(segments or ()),
        }
    )


def dumps(payload):
    return (
        json.dumps(payload, ensure_ascii=False, sort_keys=False, separators=(",", ":"))
        + "\n"
    ).encode("utf-8")


def srt_time(milliseconds):
    total = max(0, int(milliseconds))
    hours, rest = divmod(total, 3_600_000)
    minutes, rest = divmod(rest, 60_000)
    seconds, millis = divmod(rest, 1000)
    return f"{hours:02d}:{minutes:02d}:{seconds:02d},{millis:03d}"


def srt_bytes(segments):
    lines = []
    for number, segment in enumerate(segments or (), 1):
        start = int(segment["start_ms"])
        end = max(start + 1, int(segment["end_ms"]))
        lines.append(str(number))
        lines.append(f"{srt_time(start)} --> {srt_time(end)}")
        lines.append(str(segment["text"]))
        lines.append("")
    return ("\n".join(lines) + "\n").encode("utf-8") if lines else b""


def _atomic_write(path, data):
    directory = os.path.dirname(path) or "."
    os.makedirs(directory, exist_ok=True)
    handle, temporary = tempfile.mkstemp(
        prefix=".transcript-", suffix=".part", dir=directory
    )
    try:
        with os.fdopen(handle, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise
    try:
        descriptor = os.open(directory, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    except OSError:
        pass


def write(audio_path, payload, root):
    """Write the JSON then the SRT, both atomically, inside the archive."""
    real = safe_audio(audio_path, root)
    document = normalize(payload)
    target_json = json_path(real)
    target_srt = srt_path(real)
    if not inside(target_json, root) or not inside(target_srt, root):
        raise SidecarError("le sidecar sortirait de l'archive Podcasts")
    _atomic_write(target_json, dumps(document))
    _atomic_write(target_srt, srt_bytes(document["segments"]))
    return target_json


def load_file(path, root=None):
    try:
        size = os.path.getsize(path)
    except OSError as exc:
        raise SidecarError("sidecar illisible") from exc
    if size > MAX_SIDECAR_BYTES:
        raise SidecarError("sidecar trop gros")
    if root is not None and not inside(path, root):
        raise SidecarError("sidecar hors de l'archive Podcasts")
    try:
        with open(path, "rb") as handle:
            data = handle.read(MAX_SIDECAR_BYTES + 1)
    except OSError as exc:
        raise SidecarError("sidecar illisible") from exc
    if len(data) > MAX_SIDECAR_BYTES:
        raise SidecarError("sidecar trop gros")
    try:
        payload = json.loads(data.decode("utf-8"))
    except (UnicodeDecodeError, ValueError) as exc:
        raise SidecarError("sidecar JSON invalide") from exc
    return normalize(payload)


def read(audio_path, root):
    real = safe_audio(audio_path, root)
    path = json_path(real)
    if not os.path.isfile(path):
        return None
    return load_file(path, root)


def remove(audio_path, root):
    """Remove a transcript's sidecars, never the audio.

    The episode's file may already be gone (the text outlives it), so this does
    not require the audio to exist — only that every path stays under the
    podcast archive it was derived from.
    """
    if not audio_path or not root:
        raise SidecarError("archive podcast introuvable")
    try:
        real = os.path.realpath(audio_path)
    except OSError as exc:
        raise SidecarError("chemin d'épisode illisible") from exc
    if not inside(real, root):
        raise SidecarError("l'épisode est hors de l'archive Podcasts")
    for path in (json_path(real), srt_path(real)):
        if not inside(path, root):
            continue
        try:
            os.unlink(path)
        except FileNotFoundError:
            pass
    return json_path(real)
