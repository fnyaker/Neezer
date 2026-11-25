# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Seasons of a podcast, derived from what the providers actually give us.

Deezer has no season field — neither in the channel nor in an episode payload,
and there is nothing in the database to add one from. A show's own feed often
numbers its episodes in the title instead ("Saison 2", "S2E5"), so the export
groups by that number; a show that numbers nothing is grouped by publication
year, which is what a listener means by "the 2024 episodes".

The rule is deliberately all-or-nothing on a show's titles: if fewer than half
of the episodes carry a season number, the numbers are not the show's scheme
(one stray "S1E1" in a daily show would otherwise create a one-episode season
and an "everything else" bucket), and the year is the honest answer.

No database access here: callers pass plain objects with ``id``, ``title`` and
``publish_date``, so the rule is testable without the app.
"""

from __future__ import annotations

import re

# "Saison 2", "saison2", "Season 12" — a word, then the number.
_WORD = re.compile(r"\b(?:saison|season)\s*(\d{1,3})\b", re.IGNORECASE)
# "S2E5", "s02 e14" — the compact form feeds use when they have no room.
_CODE = re.compile(r"\bS(\d{1,3})\s*E\d{1,4}\b", re.IGNORECASE)

OTHER_KEY = "autres"
OTHER_LABEL = "Hors saison"
NO_DATE_KEY = "sans-date"
NO_DATE_LABEL = "Sans date"


def season_number(title):
    """The season a title announces, or None.

    The word form wins over the compact one when both appear ("Saison 2 - S1E5"
    is a show talking about its own season 2).
    """
    text = str(title or "")
    for pattern in (_WORD, _CODE):
        found = pattern.search(text)
        if found is not None:
            return int(found.group(1))
    return None


def _stamp(value):
    """A sortable text for a publish date, whatever type the caller holds.

    ``publish_date`` is a ``datetime`` from the database and may be a ``date`` in
    a caller's own fixture; both have ``strftime``. None sorts last.
    """
    if value is None:
        return None
    try:
        return value.strftime("%Y%m%d%H%M%S")
    except (AttributeError, ValueError):
        return None


def _sort_key(episode):
    stamp = _stamp(getattr(episode, "publish_date", None))
    return (stamp is None, stamp or "", str(getattr(episode, "id", "")))


def reading_order(episodes):
    """Oldest first: the order an episode's number in the ZIP follows.

    An episode with no date keeps its place at the end rather than jumping in
    front of dated ones; the id breaks ties so two same-day episodes never swap
    between two runs of the same export.
    """
    return sorted(episodes, key=_sort_key)


def _ids(episodes):
    return [str(getattr(episode, "id", "")) for episode in reading_order(episodes)]


def season_groups(episodes):
    """The show, cut into seasons: ``[{"key", "label", "episode_ids"}]``.

    Groups are newest first (a listener opens the current season first), the
    catch-all bucket last, and ``episode_ids`` inside a group is the reading
    order oldest first.
    """
    rows = list(episodes)
    numbered = []
    rest = []
    for episode in rows:
        number = season_number(getattr(episode, "title", ""))
        if number is None:
            rest.append(episode)
        else:
            numbered.append((number, episode))

    if rows and len(numbered) * 2 >= len(rows):
        bucket = {}
        for number, episode in numbered:
            bucket.setdefault(number, []).append(episode)
        groups = [
            {"key": "s%d" % number, "label": "Saison %d" % number, "episode_ids": _ids(members)}
            for number, members in sorted(bucket.items(), reverse=True)
        ]
        if rest:
            groups.append(
                {"key": OTHER_KEY, "label": OTHER_LABEL, "episode_ids": _ids(rest)}
            )
        return groups

    bucket = {}
    undated = []
    for episode in rows:
        stamp = _stamp(getattr(episode, "publish_date", None))
        if stamp is None:
            undated.append(episode)
        else:
            bucket.setdefault(stamp[:4], []).append(episode)
    groups = [
        {"key": "y" + year, "label": year, "episode_ids": _ids(members)}
        for year, members in sorted(bucket.items(), reverse=True)
    ]
    if undated:
        groups.append(
            {"key": NO_DATE_KEY, "label": NO_DATE_LABEL, "episode_ids": _ids(undated)}
        )
    return groups
