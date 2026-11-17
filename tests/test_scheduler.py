# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The auto-sync scheduler (neezer/deezer/scheduler.py).

What it must guarantee, from the resilience rules in docs/agents/server.md: a sync is
POSTPONED while Deezer is unreachable instead of failing on every entity, the
local scan still runs (it has nothing to do with Deezer), the postponement is
bounded, two syncs never run at once, and a sync that blows up never takes the
thread — or the lock — down with it. Nothing here sleeps: every wait goes
through a recorded fake.
"""

import threading
import os
import unittest
from datetime import datetime
from unittest import mock

from neezer.deezer import scheduler

from .testbase import TestBase


class _Provider:
    def __init__(self, ok=True, raises=False):
        self.ok = ok
        self.raises = raises
        self.checks = 0

    def check_login(self, force=False):
        self.checks += 1
        if self.raises:
            raise RuntimeError("boom")
        return {"ok": self.ok, "reason": None if self.ok else "network"}


class _App:
    def __init__(self, provider=None, **deezer):
        self.deezer = provider
        cfg = {"sync_user": "alice", "archive_dir": "/nowhere", "scan_local": True}
        cfg.update(deezer)
        self.config = {"DEEZER": cfg}


class ParsingTestCase(unittest.TestCase):
    def test_sync_at(self):
        self.assertEqual(scheduler._parse_at("04:00"), (4, 0))
        self.assertEqual(scheduler._parse_at("23:59"), (23, 59))
        self.assertEqual(scheduler._parse_at("7:05"), (7, 5))
        with self.assertLogs(scheduler.logger, "WARNING"):
            self.assertEqual(scheduler._parse_at("24:00"), (4, 0))
        with self.assertLogs(scheduler.logger, "WARNING"):
            self.assertEqual(scheduler._parse_at("12:60"), (4, 0))
        with self.assertLogs(scheduler.logger, "WARNING"):
            self.assertEqual(scheduler._parse_at("noon"), (4, 0))
        # Unset is not a mistake worth a warning.
        with self.assertNoLogs(scheduler.logger, "WARNING"):
            self.assertEqual(scheduler._parse_at(None), (4, 0))
            self.assertEqual(scheduler._parse_at(""), (4, 0))

    def test_a_hostile_sync_at_cannot_forge_a_log_line(self):
        # sync_at is typed in Réglages, and the value reaches the log as an
        # ARGUMENT: neezer/logsafe.py escapes the line breaks of every such
        # argument, so it cannot forge a second entry (py/log-injection).
        from neezer import logsafe

        logsafe.install()
        with self.assertLogs(scheduler.logger, "WARNING") as captured:
            self.assertEqual(
                scheduler._parse_at("04:00\nINFO  neezer.deezer.scheduler: sync ok"),
                (4, 0),
            )
        self.assertEqual(len(captured.output), 1)
        self.assertNotIn("\n", captured.output[0])
        self.assertNotIn("\r", captured.output[0])

    def _until(self, now, hour, minute):
        class Fixed(datetime):
            @classmethod
            def now(cls, tz=None):
                return now

        with mock.patch.object(scheduler, "datetime", Fixed):
            return scheduler._seconds_until(hour, minute)

    def test_seconds_until_the_next_run(self):
        at = datetime(2026, 3, 1, 3, 0, 0)
        self.assertEqual(self._until(at, 4, 0), 3600)
        # Already past today: tomorrow, never "now" and never negative.
        self.assertEqual(self._until(datetime(2026, 3, 1, 5, 0), 4, 0), 23 * 3600)
        # On the trigger minute itself: the NEXT one (the loop steps past it).
        self.assertEqual(self._until(datetime(2026, 3, 1, 4, 0), 4, 0), 24 * 3600)
        # Across a month end.
        self.assertEqual(self._until(datetime(2026, 2, 28, 23, 30), 0, 30), 3600)


class StartTestCase(unittest.TestCase):
    def _start(self, app):
        seen = []
        started = threading.Event()

        def loop(app_, schedule, on_start):
            seen.append((schedule, on_start))
            started.set()

        with mock.patch.object(scheduler, "_loop", loop):
            thread = scheduler.maybe_start(app)
            if thread is not None:
                self.assertTrue(started.wait(5))
                thread.join(5)
                self.assertTrue(thread.daemon)  # never holds up a shutdown
        return thread, seen

    def test_needs_deezer_and_a_sync_user(self):
        self.assertIsNone(self._start(_App(None))[0])
        self.assertIsNone(self._start(_App(_Provider(), sync_user=None))[0])

    def test_daily_by_default(self):
        _t, seen = self._start(_App(_Provider()))
        self.assertEqual(seen, [(("daily", (4, 0)), True)])
        _t, seen = self._start(_App(_Provider(), sync_at="21:15", sync_on_start=False))
        self.assertEqual(seen, [(("daily", (21, 15)), False)])

    def test_an_interval_wins_over_a_time(self):
        _t, seen = self._start(_App(_Provider(), sync_interval="30", sync_at="21:15"))
        self.assertEqual(seen[0][0], ("interval", 30))
        # Garbage and non-positive intervals fall back to the daily run.
        for bad in ("abc", "0", "-5"):
            _t, seen = self._start(_App(_Provider(), sync_interval=bad))
            self.assertEqual(seen[0][0], ("daily", (4, 0)), bad)


class RunSyncTestCase(TestBase):
    def setUp(self):
        super().setUp()
        self.scans = []
        self.syncs = []
        self.sweeps = []
        self.lanes = []

        test = self

        class Importer:
            def __init__(self, provider, user):
                self.user = user

            def sync(self, cfg):
                from deezerpy import _throttle

                test.syncs.append(self.user)
                test.lanes.append((os.nice(0), _throttle.current_priority()))
                if getattr(test, "sync_raises", False):
                    raise RuntimeError("deezer exploded")
                return {"tracks": 1}

        patches = [
            mock.patch("neezer.deezer.importer.DeezerImporter", Importer),
            mock.patch(
                "neezer.deezer.local.scan_local",
                lambda d: self.scans.append(d) or {"added": 0, "removed": 0},
            ),
            mock.patch(
                "neezer.deezer.backfill.sweep_for",
                lambda provider, user, **kw: self.sweeps.append(user.name) or {},
            ),
        ]
        for p in patches:
            p.start()
            self.addCleanup(p.stop)

    def tearDown(self):
        self.assertFalse(scheduler.is_syncing(), "the sync lock was left held")
        super().tearDown()

    def test_unreachable_postpones_but_still_scans(self):
        app = _App(_Provider(ok=False))
        self.assertFalse(scheduler._run_sync(app))
        self.assertEqual(self.scans, ["/nowhere"])
        self.assertEqual(self.syncs, [])
        self.assertEqual(self.sweeps, [])

    def test_a_health_check_that_raises_reads_as_unreachable(self):
        self.assertFalse(scheduler._run_sync(_App(_Provider(raises=True))))
        self.assertEqual(self.syncs, [])

    def test_reachable_syncs_then_sweeps_the_archive(self):
        self.assertTrue(scheduler._run_sync(_App(_Provider())))
        self.assertEqual(self.scans, ["/nowhere"])
        self.assertEqual(self.syncs, ["alice"])
        self.assertEqual(self.sweeps, ["alice"])

    def test_the_sweep_obeys_the_master_switch(self):
        scheduler._run_sync(_App(_Provider(), archive_library=False))
        self.assertEqual(self.syncs, ["alice"])
        self.assertEqual(self.sweeps, [])

    def test_the_local_scan_can_be_skipped(self):
        scheduler._run_sync(_App(_Provider()), scan_local=False)
        scheduler._run_sync(_App(_Provider(), scan_local=False))
        self.assertEqual(self.scans, [])
        self.assertEqual(len(self.syncs), 2)

    def test_a_failing_sync_is_contained(self):
        self.sync_raises = True
        with self.assertLogs(scheduler.logger, "WARNING"):
            # Not postponed: Deezer answered, the sync itself failed.
            self.assertTrue(scheduler._run_sync(_App(_Provider())))
        self.assertEqual(self.sweeps, [])

    def test_never_two_at_once(self):
        self.assertTrue(scheduler._sync_lock.acquire(blocking=False))
        try:
            self.assertTrue(scheduler.is_syncing())
            # Reported as done (nothing to retry) and does nothing at all.
            self.assertTrue(scheduler._run_sync(_App(_Provider())))
        finally:
            scheduler._sync_lock.release()
        self.assertEqual((self.scans, self.syncs), ([], []))

    def test_a_sync_on_a_worker_thread_runs_behind_everything_else(self):
        """The scheduler's own thread, and the one the manual route starts, are
        background work: reniced, and their Deezer calls in the bulk lane."""
        from deezerpy import _throttle

        # A thread's niceness and lane are its own for life: the main thread is
        # where the CLI and the tests run, and a sync there must touch neither.
        nice_before, lane_before = os.nice(0), _throttle.current_priority()
        self.assertTrue(scheduler._run_sync(_App(_Provider())))
        self.assertEqual((os.nice(0), _throttle.current_priority()), (nice_before, lane_before))

        thread = threading.Thread(target=scheduler._run_sync, args=(_App(_Provider()),))
        thread.start()
        thread.join(10)
        self.assertFalse(thread.is_alive())
        nice, lane = self.lanes[-1]
        self.assertGreaterEqual(nice, 10, f"the sync thread ran at nice {nice}")
        self.assertEqual(lane, _throttle.BULK)


class OutageRetryTestCase(unittest.TestCase):
    def _run(self, answers):
        calls, slept = [], []
        answers = iter(answers)

        def run_sync(app, scan_local=True):
            calls.append(scan_local)
            return next(answers)

        with mock.patch.object(scheduler, "_run_sync", run_sync), mock.patch.object(
            scheduler.time, "sleep", slept.append
        ):
            scheduler._run_when_reachable(object())
        return calls, slept

    def test_retries_until_deezer_is_back(self):
        calls, slept = self._run([False, False, True])
        # The local scan ran on the first attempt only.
        self.assertEqual(calls, [True, False, False])
        self.assertEqual(slept, [scheduler._OUTAGE_RETRY] * 2)

    def test_the_postponement_is_bounded(self):
        calls, slept = self._run([False] * 1000)
        retries = scheduler._OUTAGE_MAX_WAIT // scheduler._OUTAGE_RETRY
        self.assertEqual(len(calls), 1 + retries)
        self.assertEqual(sum(slept), scheduler._OUTAGE_MAX_WAIT)

    def test_no_retry_when_it_ran(self):
        self.assertEqual(self._run([True]), ([True], []))


if __name__ == "__main__":
    unittest.main()
