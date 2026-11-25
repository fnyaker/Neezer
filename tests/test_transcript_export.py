# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The two pure halves of the transcript export: seasons and file formats.

No database, no Flask, no engine — the rule itself, so a failure names the rule
instead of a fixture. The routes that use them are driven in
``tests/test_transcription_api.py``.
"""

import unittest
from datetime import date, datetime, timedelta

from neezer.transcribe import export, seasons


class Episode:
    """The three fields the season rule reads."""

    def __init__(self, ident, title, publish_date=None):
        self.id = ident
        self.title = title
        self.publish_date = publish_date


def day(number):
    return datetime(2026, 1, 1) + timedelta(days=number)


class SeasonNumberTestCase(unittest.TestCase):
    def test_the_word_form_and_the_compact_form(self):
        self.assertEqual(seasons.season_number("Saison 2 — Le retour"), 2)
        self.assertEqual(seasons.season_number("saison2"), 2)
        self.assertEqual(seasons.season_number("Season 12, episode 3"), 12)
        self.assertEqual(seasons.season_number("S2E5 - Le procès"), 2)
        self.assertEqual(seasons.season_number("s02 e14"), 2)

    def test_the_word_form_wins_over_the_compact_one(self):
        self.assertEqual(seasons.season_number("Saison 3 - S1E2"), 3)

    def test_no_number_is_none(self):
        for title in ("Épisode 12", "S1 with no episode", "Season finale", "", None):
            self.assertIsNone(seasons.season_number(title), title)


class SeasonGroupsTestCase(unittest.TestCase):
    def test_numbered_titles_group_by_season_newest_first(self):
        rows = [
            Episode("a", "Saison 1 - Premier", day(1)),
            Episode("b", "Saison 2 - Deuxième", day(10)),
            Episode("c", "S1E2 - Encore", day(2)),
            Episode("d", "Hors série", day(3)),
        ]
        groups = seasons.season_groups(rows)
        self.assertEqual([group["key"] for group in groups], ["s2", "s1", "autres"])
        self.assertEqual([group["label"] for group in groups], ["Saison 2", "Saison 1", "Hors saison"])
        self.assertEqual(groups[0]["episode_ids"], ["b"])
        # Reading order inside a season: oldest first, not title order.
        self.assertEqual(groups[1]["episode_ids"], ["a", "c"])
        self.assertEqual(groups[2]["episode_ids"], ["d"])

    def test_exactly_half_matching_titles_still_use_seasons(self):
        rows = [
            Episode("a", "Saison 1 - Un", day(1)),
            Episode("b", "Sans numéro", day(2)),
        ]
        groups = seasons.season_groups(rows)
        self.assertEqual([group["key"] for group in groups], ["s1", "autres"])

    def test_less_than_half_falls_back_to_the_year(self):
        rows = [
            Episode("a", "Saison 1 - Un", day(1)),
            Episode("b", "Deux", day(2)),
            Episode("c", "Trois", day(3)),
        ]
        groups = seasons.season_groups(rows)
        self.assertEqual([group["key"] for group in groups], ["y2026"])
        self.assertEqual(groups[0]["episode_ids"], ["a", "b", "c"])

    def test_year_groups_are_newest_first_with_undated_last(self):
        rows = [
            Episode("a", "Un", datetime(2024, 6, 1)),
            Episode("b", "Deux", datetime(2025, 1, 15)),
            Episode("c", "Trois", None),
            Episode("d", "Quatre", date(2025, 3, 2)),
        ]
        groups = seasons.season_groups(rows)
        self.assertEqual([group["key"] for group in groups], ["y2025", "y2024", "sans-date"])
        self.assertEqual([group["label"] for group in groups], ["2025", "2024", "Sans date"])
        self.assertEqual(groups[0]["episode_ids"], ["b", "d"])
        self.assertEqual(groups[2]["episode_ids"], ["c"])

    def test_empty_show_has_no_season(self):
        self.assertEqual(seasons.season_groups([]), [])

    def test_ids_are_strings_and_reading_order_is_stable(self):
        rows = [
            Episode(3, "Trois", day(5)),
            Episode(1, "Un", day(5)),
            Episode(2, "Deux", None),
        ]
        ordered = seasons.reading_order(rows)
        self.assertEqual([episode.id for episode in ordered], [1, 3, 2])
        groups = {group["key"]: group for group in seasons.season_groups(rows)}
        self.assertEqual(groups["y2026"]["episode_ids"], ["1", "3"])
        self.assertEqual(groups["sans-date"]["episode_ids"], ["2"])


class RenderTestCase(unittest.TestCase):
    def setUp(self):
        self.channel = type("Channel", (), {"title": "La Mémoire"})()
        self.episode = type(
            "Episode", (), {"title": "Épisode 1 — Le labo", "publish_date": datetime(2026, 3, 4)}
        )()
        self.segments = [
            {"start_ms": 0, "end_ms": 4_000, "text": "Bonjour à tous."},
            {"start_ms": 3_661_000, "end_ms": 3_665_250, "text": "La mémoire\nde travail."},
        ]

    def test_txt_carries_the_header_and_one_line_per_segment(self):
        body = export.render("txt", self.episode, self.channel, self.segments)
        self.assertEqual(
            body,
            "Titre : Épisode 1 — Le labo\n"
            "Émission : La Mémoire\n"
            "Date : 04/03/2026\n"
            "\n"
            "[00:00:00] Bonjour à tous.\n"
            "[01:01:01] La mémoire de travail.\n",
        )

    def test_txt_omits_an_unknown_date(self):
        self.episode.publish_date = None
        body = export.render("txt", self.episode, self.channel, self.segments)
        self.assertNotIn("Date", body)
        self.assertTrue(body.startswith("Titre : Épisode 1 — Le labo\nÉmission : La Mémoire\n\n"))

    def test_srt_is_numbered_and_uses_a_comma_for_milliseconds(self):
        body = export.render("srt", self.episode, self.channel, self.segments)
        self.assertEqual(
            body,
            "1\n"
            "00:00:00,000 --> 00:00:04,000\n"
            "Bonjour à tous.\n"
            "\n"
            "2\n"
            "01:01:01,000 --> 01:01:05,250\n"
            "La mémoire de travail.\n",
        )

    def test_vtt_starts_with_webvtt_and_uses_a_dot(self):
        body = export.render("vtt", self.episode, self.channel, self.segments)
        self.assertEqual(
            body,
            "WEBVTT\n"
            "\n"
            "00:00:00.000 --> 00:00:04.000\n"
            "Bonjour à tous.\n"
            "\n"
            "01:01:01.000 --> 01:01:05.250\n"
            "La mémoire de travail.\n",
        )

    def test_markdown_has_a_title_a_meta_line_and_one_paragraph_per_segment(self):
        body = export.render("md", self.episode, self.channel, self.segments)
        self.assertEqual(
            body,
            "# Épisode 1 — Le labo\n"
            "\n"
            "*La Mémoire — 04/03/2026*\n"
            "\n"
            "**[00:00:00]** Bonjour à tous.\n"
            "\n"
            "**[01:01:01]** La mémoire de travail.\n",
        )

    def test_blank_lines_and_raw_newlines_never_split_a_cue(self):
        segments = [
            {"start_ms": 0, "end_ms": 1000, "text": "Une ligne"},
            {"start_ms": 1000, "end_ms": 2000, "text": "  deux\n\nlignes\tcollées  "},
        ]
        for fmt in export.FORMATS:
            body = export.render(fmt, self.episode, self.channel, segments)
            self.assertIn("deux lignes collées", body)
            if fmt in ("srt", "vtt"):
                cues = body[len("WEBVTT\n\n") :] if fmt == "vtt" else body
                # exactly the one blank line between the two cues, never one
                # inside the second cue's text
                self.assertEqual(cues.count("\n\n"), 1, fmt)

    def test_empty_segments_are_not_written(self):
        segments = [{"start_ms": 0, "end_ms": 1000, "text": "  "}]
        self.assertEqual(export.render("srt", self.episode, self.channel, segments), "")
        self.assertEqual(export.render("vtt", self.episode, self.channel, segments), "WEBVTT\n\n")

    def test_an_unknown_format_is_refused(self):
        with self.assertRaises(ValueError):
            export.render("ass", self.episode, self.channel, self.segments)

    def test_every_format_declares_a_mime_type(self):
        self.assertEqual(set(export.FORMATS), set(export.MIME))


if __name__ == "__main__":
    unittest.main()
