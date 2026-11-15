# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Lot E of the onboarding: the account step.

What a person can actually do there (Deezer off, YouTube absent), linking the
caller's own Deezer or YouTube Music through the ``myaccounts`` engine (a
refused credential, a rate limit, no secret in a log), reading a public
playlist link as a background job whose save is the member one (no bulk
archiving), and the whole artists → account → done path. Nothing here asks a
provider to write anything.
"""

import collections
import contextlib
import json
import os
import shutil
import tempfile
import threading
import time
import unittest
from unittest import mock

import peewee

from neezer import access
from neezer import libraryimport as li
from neezer import linkimport
from neezer.config import DefaultConfig
from neezer.db import (
    Artist,
    Playlist,
    ProviderAccount,
    ServiceLink,
    StarredArtist,
    User,
    release_database,
)
from neezer.deezer import ids as dz_ids
from neezer.deezer import onboarding_pool as pool
from neezer.deezer.provider import DeezerProvider
from neezer.managers.user import UserManager, decrypt_password
from neezer.providers.errors import ProviderAuth, ProviderError, ProviderUnavailable
from neezer.web import create_application
from neezer.webui import myaccounts, onboarding_account, playlist_import
from neezer.webui.onboarding_account import onboarding_account_api
from neezer.webui.remote import POLICY

from .test_playlist_import import ImportApi, SPOTIFY_ID, sp_track, spotify_page
from .test_webui import MockDz, MockPrefetch

SECRET_KEYS = {"secret", "arl", "headers", "password", "password_clear", "token", "mac"}
SECOND_SPOTIFY_ID = "0JQ5DAqbMKFQ00XGmRA6M9"


@contextlib.contextmanager
def count_statements():
    counts = collections.Counter()
    original = peewee.Database.execute_sql
    mine = threading.get_ident()

    def execute_sql(self, sql, *args, **kwargs):
        if threading.get_ident() == mine:
            counts[sql.split(None, 1)[0].upper()] += 1
        return original(self, sql, *args, **kwargs)

    peewee.Database.execute_sql = execute_sql
    try:
        yield counts
    finally:
        peewee.Database.execute_sql = original


class OnboardingAccountTestCase(unittest.TestCase):
    def setUp(self):
        self.__db = tempfile.mkstemp()
        self.__dir = tempfile.mkdtemp()
        self.archive = tempfile.mkdtemp()
        db_path = self.__db[1]
        cache = self.__dir

        class Config(DefaultConfig):
            TESTING = True

            def __init__(self):
                super().__init__()
                self.BASE = dict(self.BASE, database_uri="sqlite:///" + db_path)
                self.WEBAPP = dict(
                    self.WEBAPP, cache_dir=cache, mount_webui=True, mount_api=True
                )

        self.app = create_application(Config())
        self._policy = dict(POLICY)
        if "onboarding_account_api" not in self.app.blueprints:
            self.app.register_blueprint(onboarding_account_api, url_prefix="/api")

        UserManager.add("alice", "Alic3", admin=True)
        UserManager.add("bob", "B0bbb")
        self.alice = User.get(name="alice")
        self.bob = User.get(name="bob")
        User.update(onboarding_step="artists").where(
            User.id.in_([self.alice.id, self.bob.id])
        ).execute()
        self.alice = User[self.alice.id]
        self.bob = User[self.bob.id]

        provider = DeezerProvider("arl", self.archive, "FLAC")
        provider._dz = MockDz()
        self.api = provider._dz.api = ImportApi()
        self.app.deezer = provider
        self.app.deezer_prefetch = MockPrefetch()
        self.app.config["DEEZER"] = dict(
            DefaultConfig.DEEZER, enabled=True, archive_dir=self.archive
        )
        self.client = self.app.test_client()

        pool.reset_state()
        playlist_import._jobs.clear()
        onboarding_account.link_limiter.clear()
        from neezer.webui import onboarding as onboarding_routes

        onboarding_routes.like_limiter.clear()

        self.pages = {}
        self._readers = (linkimport.read_spotify, linkimport.read_apple)
        linkimport.read_spotify = lambda session, link: linkimport.parse_spotify(
            self.pages[link["id"]]
        )
        linkimport.read_apple = lambda session, link: linkimport.parse_apple(
            self.pages[link["id"]]
        )

    def tearDown(self):
        linkimport.read_spotify, linkimport.read_apple = self._readers
        playlist_import._jobs.clear()
        onboarding_account.link_limiter.clear()
        li._clients.clear()
        li._listings.clear()
        li._runs.clear()
        pool.reset_state()
        POLICY.clear()
        POLICY.update(self._policy)
        release_database()
        shutil.rmtree(self.__dir, ignore_errors=True)
        shutil.rmtree(self.archive, ignore_errors=True)
        os.close(self.__db[0])
        os.remove(self.__db[1])

    # -- helpers -------------------------------------------------------------------

    def login(self, who="bob", password="B0bbb"):
        self.client.post("/api/logout")
        response = self.client.post(
            "/api/login", json={"username": who, "password": password}
        )
        self.assertEqual(response.status_code, 200, response.get_json())

    def options(self, status=200):
        response = self.client.get("/api/onboarding/account/options")
        self.assertEqual(response.status_code, status, response.get_data(as_text=True))
        return response.get_json()

    def link(self, **body):
        return self.client.post("/api/onboarding/account/link", json=body)

    def start_import(self, url, status=202):
        response = self.client.post("/api/onboarding/account/import", json={"url": url})
        self.assertEqual(response.status_code, status, response.get_data(as_text=True))
        return response

    def job(self, job_id):
        response = self.client.get(f"/api/import/playlist/{job_id}")
        self.assertEqual(response.status_code, 200, response.get_data(as_text=True))
        return response.get_json()

    def linked_row(self, provider, user=None):
        return ProviderAccount.get_or_none(
            (ProviderAccount.provider == provider)
            & (ProviderAccount.user == (user or self.bob).id)
        )

    def assert_no_secret(self, payload, *secrets):
        text = json.dumps(payload)
        for secret in secrets:
            self.assertNotIn(secret, text)

        def walk(node):
            if isinstance(node, dict):
                for key, value in node.items():
                    self.assertNotIn(str(key).lower(), SECRET_KEYS)
                    walk(value)
            elif isinstance(node, list):
                for value in node:
                    walk(value)

        walk(payload)


class AuthAndShapeTestCase(OnboardingAccountTestCase):
    def test_a_session_is_required(self):
        self.assertEqual(
            self.client.get("/api/onboarding/account/options").status_code, 401
        )
        for path, body in (
            ("/api/onboarding/account/link", {"provider": "deezer", "arl": "a" * 192}),
            ("/api/onboarding/account/import", {"url": "https://example.com/x"}),
        ):
            self.assertEqual(self.client.post(path, json=body).status_code, 401)

    def test_a_cross_site_post_is_refused(self):
        self.login()
        body = {"provider": "deezer", "arl": "a" * 192}
        for headers in (
            {"Origin": "https://evil.example"},
            {"Sec-Fetch-Site": "cross-site"},
        ):
            response = self.client.post(
                "/api/onboarding/account/link", json=body, headers=headers
            )
            self.assertEqual(response.status_code, 403, response.get_json())
        response = self.client.post(
            "/api/onboarding/account/import",
            json={"url": f"https://open.spotify.com/playlist/{SPOTIFY_ID}"},
            headers={"Origin": "https://evil.example"},
        )
        self.assertEqual(response.status_code, 403)
        self.assertEqual(playlist_import._jobs, {})
        self.assertEqual(self.options()["step"], "artists")

    def test_an_oversized_body_is_413(self):
        self.login()
        oversized = b'{"url": "' + b"a" * (9 * 1024) + b'"}'
        for path in (
            "/api/onboarding/account/link",
            "/api/onboarding/account/import",
        ):
            response = self.client.post(
                path, data=oversized, content_type="application/json"
            )
            self.assertEqual(response.status_code, 413, path)
            self.assertEqual(response.get_json()["code"], "too_large", path)
        self.assertEqual(self.options()["step"], "artists")

    def test_a_body_that_is_not_a_json_object_is_400(self):
        self.login()
        for kwargs in (
            {"data": "hello", "content_type": "text/plain"},
            {"data": "<xml/>", "content_type": "application/xml"},
            {"json": []},
            {"json": "deezer"},
        ):
            response = self.client.post("/api/onboarding/account/link", **kwargs)
            self.assertEqual(response.status_code, 400, kwargs)
            self.assertEqual(response.get_json()["code"], "invalid_body")


class OptionsTestCase(OnboardingAccountTestCase):
    def test_options_without_deezer_nor_youtube(self):
        self.app.deezer = None
        self.app.config["DEEZER"] = dict(self.app.config["DEEZER"], enabled=False)
        self.login()
        with mock.patch("neezer.deezer.youtube.installed", return_value=False):
            self.options()
            with count_statements() as counts:
                payload = self.options()

        self.assertEqual(payload["step"], "artists")
        self.assertFalse(payload["deezer"]["available"])
        self.assertFalse(payload["deezer"]["can_link"])
        self.assertFalse(payload["deezer"]["linked"])
        self.assertEqual(payload["deezer"]["auth_modes"], [])
        self.assertEqual(payload["deezer"]["reason"], "deezer_disabled")
        self.assertFalse(payload["youtube"]["available"])
        self.assertEqual(payload["youtube"]["reason"], "absent")
        self.assertEqual(payload["youtube"]["auth_modes"], [])
        by_source = {item["source"]: item for item in payload["imports"]}
        self.assertEqual(
            sorted(by_source), ["apple", "deezer", "spotify", "youtube"]
        )
        self.assertFalse(by_source["deezer"]["available"])
        self.assertEqual(by_source["deezer"]["reason"], "deezer_disabled")
        self.assertTrue(by_source["spotify"]["available"])
        self.assertTrue(by_source["apple"]["available"])
        self.assertFalse(by_source["youtube"]["available"])
        self.assertTrue(payload["can_save_imports"])
        self.assertIsNone(payload["job"])
        self.assert_no_secret(payload)
        self.assertLessEqual(sum(counts.values()), 3, counts)
        self.assertEqual(counts["SELECT"], sum(counts.values()))

    def test_options_after_linking_deezer_never_carry_the_credential(self):
        self.login()
        arl = "a" * 192
        who = {"id": "990001", "name": "Bob Dz", "lossless": True}
        with mock.patch.object(myaccounts, "_deezer_login", return_value=who):
            response = self.link(provider="deezer", arl=arl)
        self.assertEqual(response.status_code, 200, response.get_json())
        payload = response.get_json()
        self.assertEqual(payload["provider"], "deezer")
        self.assertEqual(payload["account"]["label"], "Bob Dz")
        self.assertEqual(payload["options"]["step"], "artists")
        self.assertTrue(payload["options"]["deezer"]["linked"])
        self.assertTrue(payload["options"]["deezer"]["can_link"])
        self.assert_no_secret(payload, arl)

        row = self.linked_row("deezer")
        self.assertIsNotNone(row)
        self.assertEqual(row.mode, "arl")
        self.assertNotEqual(row.secret, arl)
        self.assertEqual(decrypt_password(row.secret), arl)
        self.assertTrue(li.options(row)["lossless"])

        options = self.options()
        self.assertEqual(options["deezer"]["account"]["label"], "Bob Dz")
        self.assertEqual(options["deezer"]["account"]["mode"], "arl")
        self.assertFalse(options["deezer"]["account"]["server"])
        self.assert_no_secret(options, arl)

    def test_the_holder_of_the_server_account_cannot_link_another(self):
        self.login("alice", "Alic3")
        row = access.ensure_server_account(owner=self.alice)
        self.assertEqual(row.user_id, self.alice.id)
        options = self.options()
        self.assertTrue(options["deezer"]["linked"])
        self.assertTrue(options["deezer"]["account"]["server"])
        self.assertFalse(options["deezer"]["can_link"])
        arl = "b" * 192
        with mock.patch.object(myaccounts, "_deezer_login") as asked:
            response = self.link(provider="deezer", arl=arl)
        self.assertEqual(response.status_code, 409)
        self.assertEqual(response.get_json()["code"], "server_account_holder")
        asked.assert_not_called()

    def test_options_report_the_import_already_running(self):
        self.login()
        url = f"https://open.spotify.com/playlist/{SPOTIFY_ID}"
        with mock.patch.object(playlist_import, "_run_import", lambda *a, **k: None):
            started = self.start_import(url).get_json()
            options = self.options()
        self.assertIsNotNone(options["job"])
        self.assertEqual(options["job"]["id"], started["job"]["id"])
        self.assertEqual(options["job"]["state"], "reading")
        self.assertEqual(options["job"]["source"], "spotify")
        self.assertEqual(options["job"]["source_label"], "Spotify")


class LinkTestCase(OnboardingAccountTestCase):
    def test_a_malformed_arl_is_refused_before_asking_deezer(self):
        self.login()
        with mock.patch.object(
            myaccounts, "_deezer_login", side_effect=AssertionError("asked Deezer")
        ) as asked:
            for arl in ("", "abc-def", "a" * 600, "a" * 191 + "!"):
                response = self.link(provider="deezer", arl=arl)
                self.assertEqual(response.status_code, 400, arl)
                self.assertEqual(response.get_json()["code"], "invalid_arl", arl)
            asked.assert_not_called()
        response = self.link(provider="spotify", arl="a" * 192)
        self.assertEqual(response.status_code, 400)
        self.assertEqual(response.get_json()["code"], "invalid_provider")

    def test_a_refused_credential_says_why_and_never_blocks_the_step(self):
        self.login()
        arl = "c" * 192
        with mock.patch.object(
            myaccounts, "_deezer_login", side_effect=ProviderAuth("refused", provider="deezer")
        ):
            response = self.link(provider="deezer", arl=arl)
        self.assertEqual(response.status_code, 400)
        self.assertEqual(response.get_json()["code"], "provider_refused")
        self.assertIn("Deezer refuse", response.get_json()["error"])

        with mock.patch.object(
            myaccounts,
            "_deezer_login",
            side_effect=ProviderUnavailable("down", provider="deezer"),
        ):
            response = self.link(provider="deezer", arl=arl)
        self.assertEqual(response.status_code, 503)
        self.assertEqual(response.get_json()["code"], "provider_unreachable")
        self.assertTrue(response.headers.get("Retry-After"))

        self.assertIsNone(self.linked_row("deezer"))
        self.assertEqual(self.options()["step"], "artists")

    def test_a_provider_error_never_puts_the_credential_in_a_log_or_a_body(self):
        self.login()
        arl = "d" * 192
        error = ProviderError(f"deezer said no for {arl}", provider="deezer")
        with mock.patch.object(myaccounts, "_deezer_login", side_effect=error):
            with self.assertLogs(level="INFO") as logs:
                response = self.link(provider="deezer", arl=arl)
        self.assertEqual(response.status_code, 502)
        self.assertEqual(response.get_json()["code"], "provider_error")
        self.assertNotIn(arl, response.get_data(as_text=True))
        self.assertNotIn(arl, "\n".join(logs.output))

    def test_linking_is_rate_limited(self):
        self.login()
        with mock.patch.object(
            myaccounts,
            "_deezer_login",
            side_effect=ProviderAuth("refused", provider="deezer"),
        ) as asked:
            for _ in range(onboarding_account.link_limiter.max_events):
                self.assertEqual(self.link(provider="deezer", arl="e" * 192).status_code, 400)
            response = self.link(provider="deezer", arl="e" * 192)
        self.assertEqual(response.status_code, 429)
        self.assertEqual(response.get_json()["code"], "rate_limited")
        self.assertTrue(response.headers.get("Retry-After"))
        self.assertEqual(asked.call_count, onboarding_account.link_limiter.max_events)

    def test_youtube_headers_are_verified_stored_encrypted_and_never_echoed(self):
        self.login()
        raw = "cookie: SECRET-COOKIE\nx-goog-authuser: 0"
        headers_json = '{"cookie": "SECRET-COOKIE", "x-goog-authuser": "0"}'

        class FakeClient:
            def __init__(self, data):
                self.data = data

            def account(self):
                return "Léa YT"

        with mock.patch("neezer.deezer.youtube.installed", return_value=True), \
                mock.patch("neezer.services.ytmusic.parse_headers", return_value=headers_json), \
                mock.patch("neezer.services.ytmusic.Client", FakeClient), \
                mock.patch.object(li, "start", return_value=True) as started:
            response = self.link(provider="youtube", headers=raw)
        self.assertEqual(response.status_code, 200, response.get_json())
        payload = response.get_json()
        self.assertEqual(payload["account"]["label"], "Léa YT")
        self.assertTrue(payload["options"]["youtube"]["linked"])
        self.assertEqual(payload["options"]["youtube"]["auth_modes"], ["headers"])
        self.assert_no_secret(payload, raw, headers_json, "SECRET-COOKIE")

        link = ServiceLink.get(ServiceLink.service == "youtube")
        self.assertEqual(decrypt_password(link.data), headers_json)
        self.assertEqual(link.account, "Léa YT")
        started.assert_called_once()

    def test_youtube_errors_are_the_service_s_in_words(self):
        self.login()
        from neezer.services import ServiceError

        with mock.patch("neezer.deezer.youtube.installed", return_value=True), \
                mock.patch(
                    "neezer.services.ytmusic.parse_headers",
                    side_effect=ServiceError("En-têtes incomplets : il faut le cookie."),
                ):
            response = self.link(provider="youtube", headers="x")
        self.assertEqual(response.status_code, 400)
        self.assertEqual(response.get_json()["code"], "invalid_headers")
        self.assertIn("En-têtes incomplets", response.get_json()["error"])

        with mock.patch("neezer.deezer.youtube.installed", return_value=False):
            response = self.link(provider="youtube", headers="x")
        self.assertEqual(response.status_code, 400)
        self.assertEqual(response.get_json()["code"], "absent")

    def test_deezer_switched_off_links_nothing(self):
        self.app.deezer = None
        self.app.config["DEEZER"] = dict(self.app.config["DEEZER"], enabled=False)
        self.login()
        with mock.patch.object(myaccounts, "_deezer_login") as asked:
            response = self.link(provider="deezer", arl="f" * 192)
        self.assertEqual(response.status_code, 400)
        self.assertEqual(response.get_json()["code"], "deezer_disabled")
        asked.assert_not_called()


class ImportTestCase(OnboardingAccountTestCase):
    def _page(self):
        self.pages[SPOTIFY_ID] = spotify_page(
            [
                sp_track("Alors on danse", "Stromae", 207000),
                sp_track("Bella", "Maître Gims", 231000),
            ]
        )

    def _gw_get_track(self, sid):
        titles = {"1": ("Alors on danse", 207), "3": ("Bella", 231)}
        title, duration = titles.get(str(sid), ("Titre", 200))
        return {
            "SNG_ID": str(sid),
            "SNG_TITLE": title,
            "DURATION": duration,
            "ART_ID": 30,
            "ART_NAME": "Stromae",
            "ALB_ID": 300,
            "ALB_TITLE": "Album",
            "ALB_PICTURE": "",
            "TRACK_NUMBER": 1,
            "DISK_NUMBER": 1,
        }

    def test_a_public_playlist_is_a_job_then_a_member_save_without_archiving(self):
        self.login()
        self._page()
        forbidden = {}
        for name in (
            "create_playlist",
            "add_playlist_tracks",
            "edit_playlist",
            "delete_playlist",
            "add_tracks",
            "remove_tracks",
        ):
            forbidden[name] = mock.Mock(side_effect=AssertionError(name))
            setattr(self.app.deezer._dz.gw, name, forbidden[name])

        response = self.start_import(
            f"https://open.spotify.com/playlist/{SPOTIFY_ID}?si=tracking"
        )
        payload = response.get_json()
        job = payload["job"]
        self.assertEqual(payload["id"], job["id"])
        self.assertTrue(job["id"])
        self.assertEqual(job["state"], "done", job)
        self.assertEqual(job["source"], "spotify")
        self.assertEqual(job["source_label"], "Spotify")
        self.assertEqual((job["total"], job["matched"]), (2, 2), job)
        self.assertEqual(job["counts"], {"deezer": 2})
        self.assertEqual([line["track"]["deezer_id"] for line in job["items"]], ["1", "3"])
        self.assertIsNone(payload["options"]["job"])
        self.assertEqual(payload["options"]["step"], "artists")

        polled = self.job(job["id"])
        self.assertEqual(polled["state"], "done")
        self.assertEqual(polled["items"], job["items"])

        self.app.deezer._dz.gw.get_track = self._gw_get_track
        with mock.patch("neezer.deezer.backfill.archive_tracks") as archive:
            saved = self.client.post(f"/api/import/playlist/{job['id']}/save", json={})
            self.assertEqual(saved.status_code, 200, saved.get_json())
            archive.assert_not_called()
        finished = self.job(job["id"])
        self.assertEqual(finished["state"], "saved", finished)
        playlist = Playlist[finished["playlist_id"]]
        self.assertEqual(playlist.user_id, self.bob.id)
        self.assertIsNone(playlist.deezer_id)
        self.assertEqual(len(playlist.get_tracks()), 2)

        for name, spy in forbidden.items():
            spy.assert_not_called()
        self.assertEqual(self.app.deezer_prefetch.ids, [])
        self.assertEqual(self.app.deezer_prefetch.episode_ids, [])

        self.login("alice", "Alic3")
        self.assertEqual(
            self.client.get(f"/api/import/playlist/{job['id']}").status_code, 404
        )
        self.assertEqual(
            self.client.post(f"/api/import/playlist/{job['id']}/save", json={}).status_code,
            404,
        )

    def test_a_playlist_it_cannot_read_says_why(self):
        self.login()
        self.pages[SPOTIFY_ID] = "<html>nothing here</html>"
        payload = self.start_import(
            f"https://open.spotify.com/playlist/{SPOTIFY_ID}"
        ).get_json()
        self.assertEqual(payload["job"]["state"], "error")
        self.assertIn("Spotify", payload["job"]["error"])
        self.assertIsNone(payload["options"]["job"])

    def test_a_link_it_cannot_read_is_refused_without_a_job(self):
        self.login()
        for url, code in (
            (None, "missing_url"),
            ("   ", "missing_url"),
            (5, "missing_url"),
            ("https://example.com/playlist/1", "invalid_link"),
            ("javascript:alert(1)//open.spotify.com/playlist/" + SPOTIFY_ID, "invalid_link"),
            ("https://open.spotify.com/playlist/short", "invalid_link"),
        ):
            response = self.client.post(
                "/api/onboarding/account/import", json={"url": url} if url is not None else {}
            )
            self.assertEqual(response.status_code, 400, url)
            self.assertEqual(response.get_json()["code"], code, url)
        self.assertEqual(playlist_import._jobs, {})

    def test_an_import_already_running_is_429_with_a_cause(self):
        self.login()
        with mock.patch.object(playlist_import, "_run_import", lambda *a, **k: None):
            first = self.start_import(
                f"https://open.spotify.com/playlist/{SPOTIFY_ID}"
            ).get_json()
            self.start_import(f"https://open.spotify.com/playlist/{SECOND_SPOTIFY_ID}")
            response = self.client.post(
                "/api/onboarding/account/import",
                json={"url": f"https://open.spotify.com/playlist/{SPOTIFY_ID}"},
            )
        self.assertEqual(response.status_code, 429)
        self.assertEqual(response.get_json()["code"], "import_running")
        self.assertTrue(response.headers.get("Retry-After"))
        options = self.options()
        self.assertNotEqual(options["job"]["id"], first["job"]["id"])

    def test_starting_an_import_does_not_hold_the_request(self):
        self.login()
        release = threading.Event()
        started = threading.Event()
        finished = threading.Event()

        def slow(*_args, **_kwargs):
            started.set()
            release.wait(5)
            finished.set()

        self.app.testing = False
        try:
            with mock.patch.object(playlist_import, "_run_import", slow):
                began = time.monotonic()
                self.start_import(f"https://open.spotify.com/playlist/{SPOTIFY_ID}")
                elapsed = time.monotonic() - began
                self.assertTrue(started.wait(2))
                self.assertLess(elapsed, 1.0)
        finally:
            release.set()
            self.assertTrue(finished.wait(5))
            self.app.testing = True

    def test_a_deezer_link_needs_the_links_grant_of_that_person(self):
        self.login("alice", "Alic3")
        row = access.ensure_server_account(owner=self.alice)
        closed = self.client.put(
            "/api/access/grants",
            json={"provider": "deezer", "account": "", "feature": "*", "subjects": []},
        )
        self.assertEqual(closed.status_code, 200, closed.get_json())
        granted = self.client.put(
            "/api/access/grants",
            json={
                "provider": "deezer",
                "account": str(row.id),
                "feature": "links",
                "subjects": ["u:" + str(self.alice.id)],
            },
        )
        self.assertEqual(granted.status_code, 200, granted.get_json())

        self.login()
        options = self.options()
        deezer_import = {i["source"]: i for i in options["imports"]}["deezer"]
        self.assertFalse(deezer_import["available"])
        self.assertEqual(deezer_import["reason"], "deezer_denied")
        response = self.client.post(
            "/api/onboarding/account/import",
            json={"url": "https://www.deezer.com/fr/playlist/1479458365"},
        )
        self.assertEqual(response.status_code, 403)
        self.assertEqual(response.get_json()["code"], "deezer_denied")
        self.assertEqual(playlist_import._jobs, {})

        self.login("alice", "Alic3")
        accepted = self.start_import("https://www.deezer.com/fr/playlist/1479458365")
        self.assertEqual(accepted.get_json()["job"]["source"], "deezer")


class FullPathTestCase(OnboardingAccountTestCase):
    def test_the_account_step_blocks_nothing(self):
        self.login()
        User.update(onboarding_step="account").where(User.id == self.bob.id).execute()
        finished = self.client.post("/api/onboarding/account/done")
        self.assertEqual(finished.status_code, 200, finished.get_json())
        self.assertEqual(finished.get_json()["step"], "done")
        self.assertIsNone(self.options()["deezer"]["account"])
        self.assertIsNone(self.options()["youtube"]["account"])
        self.assertIsNone(self.options()["job"])

    def test_artists_then_account_then_done(self):
        self.login()
        artist = Artist.create(
            id=dz_ids.artist_uuid("555"), name="Local 555", deezer_id="555"
        )
        liked = self.client.post("/api/onboarding/artists/like", json={"deezer_id": "555"})
        self.assertEqual(liked.status_code, 200, liked.get_json())
        self.assertTrue(
            StarredArtist.get_or_none(
                (StarredArtist.user == self.bob) & (StarredArtist.starred == artist.id)
            )
        )

        account_done = self.client.post("/api/onboarding/account/done")
        self.assertEqual(account_done.status_code, 409)
        self.assertEqual(account_done.get_json()["code"], "artists_not_done")

        artists_done = self.client.post("/api/onboarding/artists/done")
        self.assertEqual(artists_done.status_code, 200, artists_done.get_json())
        self.assertEqual(artists_done.get_json()["step"], "account")
        self.assertEqual(self.options()["step"], "account")

        arl = "g" * 192
        with mock.patch.object(
            myaccounts,
            "_deezer_login",
            return_value={"id": "990002", "name": "Bob Dz", "lossless": False},
        ):
            linked = self.link(provider="deezer", arl=arl)
        self.assertEqual(linked.status_code, 200, linked.get_json())
        self.assertEqual(linked.get_json()["options"]["step"], "account")

        finished = self.client.post("/api/onboarding/account/done")
        self.assertEqual(finished.status_code, 200, finished.get_json())
        self.assertEqual(finished.get_json()["step"], "done")
        self.assertEqual(finished.get_json()["user"]["onboarding"], "done")
        self.assertEqual(User[self.bob.id].onboarding_step, "done")
        self.assertEqual(self.options()["step"], "done")

        again = self.client.post("/api/onboarding/account/done")
        self.assertEqual(again.status_code, 200)
        self.assertEqual(again.get_json()["step"], "done")
