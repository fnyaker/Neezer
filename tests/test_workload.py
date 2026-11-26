# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""A person waiting in front of the app outranks every background job.

neezer/deezer/workload.py and the download queue in prefetch.py are how
that rule is kept, and until now only the request admission (503 +
Retry-After) was tested. The rest — the priority order of the queue, bulk work
standing down while a foreground download runs, the outage wait, the queue's
bound — ran in no test at all. These drive the real DeezerPrefetcher with the
archiving functions replaced by recorders, so what is asserted is the order
and timing the queue really produces.
"""

import os
import threading
import time
import unittest
from unittest import mock

from deezerpy.errors import DeezerUnavailable

from neezer.deezer import prefetch, workload
from neezer.deezer.workload import Priority

# How long a test waits for a worker before calling it stuck. Never reached by
# a passing test; generous so a loaded CI machine costs nothing.
WAIT = 10.0


class PriorityNamesTestCase(unittest.TestCase):
    def test_the_client_names_a_reason_never_a_number(self):
        self.assertEqual(workload.priority_from_name("user"), Priority.USER)
        self.assertEqual(workload.priority_from_name(" Prefetch "), Priority.PREFETCH)
        self.assertEqual(workload.priority_from_name("BULK"), Priority.BULK)
        # Unknown, empty, or an attempt to claim the front by number: bulk.
        for claim in ("urgent", "", None, "0", 0, "-1"):
            self.assertEqual(workload.priority_from_name(claim), Priority.BULK, claim)
        self.assertLess(Priority.USER, Priority.PREFETCH)
        self.assertLess(Priority.PREFETCH, Priority.BULK)


class SizingTestCase(unittest.TestCase):
    def _cores(self, n):
        return mock.patch.object(workload.os, "sched_getaffinity", lambda pid: set(range(n)))

    def test_a_batch_job_takes_half_the_box_and_never_the_last_core(self):
        # cores -> workers
        for cores, want in ((1, 1), (2, 1), (3, 1), (4, 2), (8, 4), (16, 8), (64, 8)):
            with self._cores(cores):
                self.assertEqual(workload.cpu_workers(), want, cores)
        # Asked for the whole box, it still leaves one core to the server.
        for cores, want in ((1, 1), (2, 1), (4, 3), (8, 7)):
            with self._cores(cores):
                self.assertEqual(workload.cpu_workers(share=1.0), want, cores)

    def test_background_requests_get_a_quarter_of_the_pool(self):
        for threads, want in (("16", 4), ("8", 2), ("4", 1), ("2", 1), ("64", 4)):
            with mock.patch.dict(os.environ, {"GUNICORN_THREADS": threads}, clear=False):
                os.environ.pop("NS_BACKGROUND_SLOTS", None)
                self.assertEqual(workload._default_background_slots(), want, threads)
        # The operator can say otherwise; garbage is ignored, not obeyed.
        with mock.patch.dict(os.environ, {"GUNICORN_THREADS": "16", "NS_BACKGROUND_SLOTS": "7"}):
            self.assertEqual(workload._default_background_slots(), 7)
        with mock.patch.dict(os.environ, {"GUNICORN_THREADS": "junk", "NS_BACKGROUND_SLOTS": "-3"}):
            self.assertEqual(workload._default_background_slots(), 4)  # 16 // 4


class QuietWaitTestCase(unittest.TestCase):
    def test_no_foreground_work_means_no_wait(self):
        self.assertFalse(workload.foreground_busy())
        t0 = time.monotonic()
        workload.quiet_wait(5.0)
        self.assertLess(time.monotonic() - t0, 0.1)

    def test_it_waits_for_the_foreground_to_finish_not_for_the_timeout(self):
        release = threading.Event()
        entered = threading.Event()

        def person_waiting():
            with workload.foreground():
                entered.set()
                release.wait(WAIT)

        t = threading.Thread(target=person_waiting)
        t.start()
        self.assertTrue(entered.wait(WAIT))
        self.assertTrue(workload.foreground_busy())
        threading.Timer(0.15, release.set).start()
        t0 = time.monotonic()
        workload.quiet_wait(5.0)
        waited = time.monotonic() - t0
        t.join(WAIT)
        self.assertGreater(waited, 0.1)  # it did wait...
        self.assertLess(waited, 2.0)  # ...for the foreground, not the 5 s cap
        self.assertFalse(workload.foreground_busy())

    def test_a_wedged_foreground_only_slows_the_bulk_queue_down(self):
        with workload.foreground():
            t0 = time.monotonic()
            workload.quiet_wait(0.3)
            self.assertLess(abs((time.monotonic() - t0) - 0.3), 0.25)

    def test_an_exception_in_the_foreground_still_releases_it(self):
        with self.assertRaises(ZeroDivisionError):
            with workload.foreground():
                1 / 0
        self.assertFalse(workload.foreground_busy())


class DownloadQueueTestCase(unittest.TestCase):
    """The real queue and worker, with the archiving itself recorded."""

    def setUp(self):
        self.done = []
        self.calls = []
        self.errors = {}  # deezer id -> exceptions to raise, in order
        self.cond = threading.Condition()
        self.gate = threading.Event()
        self.gate.set()

        test = self

        class Track:
            def __init__(self, did):
                self.deezer_id = did

        def import_track(provider, did):
            return Track(did)

        def ensure_archived(provider, track):
            did = track.deezer_id
            with test.cond:
                test.calls.append(did)
            if did == "gate":
                test.gate.wait(WAIT)
            errors = test.errors.get(did)
            if errors:
                raise errors.pop(0)
            with test.cond:
                test.done.append(did)
                test.cond.notify_all()

        # The worker imports these when its thread starts, so they are patched
        # before the prefetcher exists.
        for name, fake in (
            ("import_track", import_track),
            ("ensure_archived", ensure_archived),
            ("find_local_track", lambda did: None),
        ):
            p = mock.patch(f"neezer.deezer.archive.{name}", fake)
            p.start()
            self.addCleanup(p.stop)
        # prefetch's OWN clock: patching time.sleep itself would also turn the
        # waits in these tests into no-ops (it is one module object).
        self.slept = []
        fake_time = mock.Mock(wraps=time)
        fake_time.sleep = self.slept.append
        p = mock.patch.object(prefetch, "time", fake_time)
        p.start()
        self.addCleanup(p.stop)
        p = mock.patch.object(prefetch, "renice", lambda delta=10: None)
        p.start()
        self.addCleanup(p.stop)

    def _prefetcher(self, **kw):
        kw.setdefault("workers", 1)
        kw.setdefault("dl_workers", 1)
        pf = prefetch.DeezerPrefetcher(provider=object(), **kw)
        self.addCleanup(self._stop, pf)
        return pf

    def _stop(self, pf):
        self.gate.set()
        for _ in pf._workers:
            pf._dl_queue.put((99, 10**9, None))  # the worker's own stop marker

    def _until(self, predicate, what, within=WAIT):
        deadline = time.monotonic() + within
        with self.cond:
            while not predicate():
                left = deadline - time.monotonic()
                if left <= 0:
                    self.fail(f"timed out waiting for {what}: done={self.done}")
                self.cond.wait(min(left, 0.05))

    def test_a_person_goes_ahead_of_a_discography(self):
        """Pressing play during an artist archive used to wait behind it."""
        pf = self._prefetcher()
        self.gate.clear()
        pf.download_ids(["gate"], Priority.BULK)  # occupies the only worker
        self._until(lambda: self.calls == ["gate"], "the first item to start")
        pf.download_ids(["b1", "b2", "b3"], Priority.BULK)
        pf.download_ids(["p1"], Priority.PREFETCH)
        pf.download_ids(["u1"], Priority.USER)
        pf.download_ids(["b4"], Priority.BULK)
        self.gate.set()
        self._until(lambda: len(self.done) == 7, "the queue to drain")
        # Most urgent first; arrival order within one priority.
        self.assertEqual(self.done, ["gate", "u1", "p1", "b1", "b2", "b3", "b4"])

    def test_bulk_work_stands_down_while_somebody_waits(self):
        pf = self._prefetcher(dl_workers=2)
        with workload.foreground():
            pf.download_ids(["bulk"], Priority.BULK)
            pf.download_ids(["user"], Priority.USER)
            pf.download_ids(["next"], Priority.PREFETCH)
            # Only the person's own tap (USER) goes ahead; everything below it —
            # including the next track's preload — waits while the stream they
            # are hearing still needs the link.
            self._until(lambda: "user" in self.done, "the user's own download", within=2.0)
            time.sleep(0.3)
            self.assertNotIn("next", self.done, "a PREFETCH competed with a person's stream")
            self.assertNotIn("bulk", self.done)
        self._until(
            lambda: {"next", "bulk"} <= set(self.done),
            "the preload and the bulk download once it is quiet",
            within=2.0,
        )

    def test_prefetch_yields_to_the_track_the_person_is_hearing(self):
        """The next track is preloaded at PREFETCH; while somebody's own stream
        is downloading it must stand down, not split the one link."""
        pf = self._prefetcher()
        release = threading.Event()

        def person_waiting():
            with workload.foreground():
                release.wait(WAIT)

        t = threading.Thread(target=person_waiting)
        t.start()
        try:
            pf.download_ids(["next"], Priority.PREFETCH)
            time.sleep(0.4)
            self.assertEqual(
                self.calls,
                [],
                "a PREFETCH download started while the person's stream downloaded",
            )
        finally:
            release.set()
            t.join(WAIT)
        self._until(
            lambda: "next" in self.done,
            "the preload once the stream is done",
            within=2.0,
        )

    def test_an_outage_is_waited_out_not_counted_as_a_failure(self):
        pf = self._prefetcher()
        self.errors["t1"] = [DeezerUnavailable("down"), DeezerUnavailable("down")]
        waits = iter([12.0, 90.0])
        with mock.patch.object(prefetch, "_outage_wait", lambda: next(waits)):
            pf.download_ids(["t1"])
            self._until(lambda: "t1" in self.done, "the retried download")
        self.assertEqual(self.calls, ["t1", "t1", "t1"])
        # The breaker's own cooldown, capped at _MAX_OUTAGE_WAIT.
        self.assertEqual(self.slept, [12.0, prefetch._MAX_OUTAGE_WAIT])

    def test_a_real_failure_is_not_retried(self):
        pf = self._prefetcher()
        self.errors["bad"] = [ValueError("no such track")]
        with mock.patch.object(prefetch, "_outage_wait", lambda: 30.0):
            pf.download_ids(["bad", "next"])
            self._until(lambda: "next" in self.done, "the item after the failure")
        self.assertEqual(self.calls, ["bad", "next"])
        self.assertEqual(self.slept, [])

    def test_a_transport_failure_without_an_open_circuit_is_not_retried(self):
        # e.g. a podcast host that will not resolve: nothing says a retry in a
        # second would land differently.
        pf = self._prefetcher()
        self.errors["host"] = [DeezerUnavailable("no route")]
        with mock.patch.object(prefetch, "_outage_wait", lambda: 0.0):
            pf.download_ids(["host", "next"])
            self._until(lambda: "next" in self.done, "the next item")
        self.assertEqual(self.calls, ["host", "next"])
        self.assertEqual(self.slept, [])

    def test_the_retries_are_bounded(self):
        pf = self._prefetcher()
        self.errors["t"] = [DeezerUnavailable("down")] * 50
        with mock.patch.object(prefetch, "_outage_wait", lambda: 1.0):
            pf.download_ids(["t", "after"])
            self._until(lambda: "after" in self.done, "the item after an outage")
        # A literal, not _MAX_OUTAGE_RETRIES + 1: the first attempt and three
        # retries, then the nightly sweep's job.
        self.assertEqual(self.calls.count("t"), 4)

    def test_a_full_queue_refuses_and_says_how_many_it_took(self):
        """Every entry is a FLAC on disk; an unbounded queue was a way to fill it."""
        pf = self._prefetcher(max_download_queue=3)
        self.gate.clear()
        pf.download_ids(["gate"])
        self._until(lambda: self.calls == ["gate"], "the worker to be busy")
        self.assertEqual(pf.download_ids(["a", "b", "c", "d", "e"]), 3)
        self.assertEqual(pf.download_pending, 3)
        self.assertEqual(pf.download_episode_ids(["x"]), 0)


class AnalysisThreadNiceTest(unittest.TestCase):
    """The per-track measurement queued at the end of a FIRST PLAY runs ffmpeg
    and the beat tracker while the same box transcodes that play: it has to
    run behind it, like every other background job (it was the one that did
    not)."""

    def test_the_measurement_thread_is_reniced_before_it_works(self):
        import threading
        import uuid as _uuid

        from neezer.deezer import analysis, workload

        order = []
        done = threading.Event()

        def fake_analyze(track, provider=None):
            order.append(("analyze", threading.get_ident()))
            done.set()

        def fake_renice(delta=10):
            order.append(("renice", threading.get_ident(), delta))

        track = mock.Mock(id=_uuid.uuid4())
        with mock.patch.object(analysis, "_under_test", lambda: False), mock.patch.object(
            analysis, "ffmpeg_available", lambda: True
        ), mock.patch.object(analysis, "analyze_track", fake_analyze), mock.patch.object(
            workload, "renice", fake_renice
        ):
            analysis.queue_analysis(track)
            self.assertTrue(done.wait(5))
        self.assertEqual(order[0][0], "renice")
        self.assertEqual(order[0][2], 12)
        # On the worker thread, never the request thread that queued it.
        self.assertNotEqual(order[0][1], threading.get_ident())
        self.assertEqual(order[0][1], order[1][1])


class ThrottleTestCase(unittest.TestCase):
    """The zero-impact gate library maintenance goes through (workload.Throttle):
    nothing runs while anybody listens or a download is awaited, the machine is
    left a clear share, and a cancelled job leaves without finishing a unit."""

    def setUp(self):
        self.slept = []
        workload._last_listen = 0.0
        self.addCleanup(setattr, workload, "_last_listen", 0.0)

    def _throttle(self, **kw):
        kw.setdefault("sleep", self.slept.append)
        return workload.Throttle(**kw)

    def test_an_idle_machine_is_not_waited_for(self):
        with mock.patch.object(workload, "load_per_core", lambda: 0.0):
            self.assertTrue(self._throttle().wait())
        self.assertEqual(self.slept, [])

    def test_it_waits_for_a_listener_then_goes(self):
        workload.touch_listening()
        n = []

        def sleep(s):
            n.append(s)
            workload._last_listen -= 100  # the song ended

        t = workload.Throttle(sleep=sleep, listen_quiet=45.0)
        with mock.patch.object(workload, "load_per_core", lambda: 0.0):
            self.assertTrue(t.wait())
        self.assertEqual(len(n), 1)

    def test_it_waits_for_a_download_a_person_is_waiting_on(self):
        t = self._throttle()
        seen = []
        with mock.patch.object(workload, "load_per_core", lambda: 0.0):
            with workload.foreground():
                t._sleep = lambda s: (seen.append(t.reason), setattr(t, "_stop", lambda: True))
                self.assertFalse(t.wait())  # cancelled while paused
        self.assertEqual(seen, ["download"])

    def test_a_loaded_box_is_left_alone(self):
        t = self._throttle()
        seen = []
        t._sleep = lambda s: (seen.append(t.reason), setattr(t, "_stop", lambda: True))
        with mock.patch.object(workload, "load_per_core", lambda: 3.0):
            self.assertFalse(t.wait())
        self.assertEqual(seen, ["load"])

    def test_a_cancelled_job_does_not_start_another_unit(self):
        t = self._throttle(stop=lambda: True)
        self.assertFalse(t.wait())

    def test_the_duty_cycle_caps_the_share_of_time_taken(self):
        t = self._throttle(duty=0.5)
        t.spent(2.0)
        self.assertEqual(self.slept, [2.0])  # worked 2 s, rests 2 s: half
        self.slept.clear()
        t = self._throttle(duty=0.25)
        t.spent(1.0)
        self.assertEqual(self.slept, [3.0])
        self.slept.clear()
        self._throttle(duty=1.0).spent(5.0)
        self.assertEqual(self.slept, [])  # full speed: no rest


class SweepYieldTestCase(unittest.TestCase):
    """The archive sweep — the nightly sync's and the admin button's — is
    background work like every other: its thread is reniced, and between two
    items it stands aside while somebody waits on a download."""

    def setUp(self):
        workload.reset_streams()
        self.addCleanup(workload.reset_streams)
        workload._last_listen = 0.0
        self.addCleanup(setattr, workload, "_last_listen", 0.0)

    def test_it_yields_between_two_items_and_runs_reniced(self):
        from neezer.deezer import backfill

        calls = []
        second = threading.Event()
        cond = threading.Condition()
        nice = []

        def ensure_archived(provider, track):
            with cond:
                calls.append((track.deezer_id, workload.foreground_busy()))
                nice.append(os.nice(0))
                cond.notify_all()
            if track.deezer_id == "2":
                second.set()

        tracks = [mock.Mock(deezer_id="1"), mock.Mock(deezer_id="2")]
        with mock.patch("neezer.deezer.archive.ensure_archived", ensure_archived):
            with workload.foreground():
                thread = threading.Thread(target=backfill.run, args=(None, tracks, []))
                thread.start()
                with cond:
                    while not calls:
                        self.assertTrue(cond.wait(10), "the sweep never started")
                # The first item is already archived; the second must NOT begin
                # while somebody is waiting — 5 polls of the gate's 0.2 s.
                self.assertFalse(
                    second.wait(1.0), "the sweep took the link from a waiting listener"
                )
            self.assertTrue(second.wait(10), "the sweep never resumed")
            thread.join(10)
            self.assertFalse(thread.is_alive())
        self.assertEqual([c[0] for c in calls], ["1", "2"])
        self.assertEqual([c[1] for c in calls], [True, False])
        self.assertGreaterEqual(min(nice), 10, f"the sweep thread ran at nice {nice}")


class DeezerLaneTestCase(unittest.TestCase):
    """Nothing is a throttle, only an order: the shared Deezer window serves a
    person's call before a background one when the next slot frees."""

    def setUp(self):
        from deezerpy import _throttle

        self.throttle = _throttle
        self.addCleanup(_throttle.set_priority, _throttle.USER)

    def test_the_shared_window_is_still_forty_calls_per_five_seconds(self):
        self.assertEqual(self.throttle.limiter.max_calls, 40)
        self.assertEqual(self.throttle.limiter.period, 5.0)

    def test_forty_bulk_calls_then_a_person_goes_first(self):
        limiter = self.throttle.RateLimiter(max_calls=40, period=0.3)
        for _ in range(40):
            limiter.acquire(self.throttle.BULK)  # the window is now full
        order = []

        def background():
            limiter.acquire(self.throttle.BULK)
            order.append("bulk")

        def person():
            limiter.acquire()  # this thread's own lane: USER
            order.append("user")

        bulk = threading.Thread(target=background)
        bulk.start()
        self._until(lambda: limiter.waiting(self.throttle.BULK) == 1, "the bulk waiter")
        user = threading.Thread(target=person)
        user.start()
        self._until(lambda: limiter.waiting(self.throttle.USER) == 1, "the person's call")
        bulk.join(10)
        user.join(10)
        self.assertEqual(order, ["user", "bulk"], f"served in the order {order}")

    def test_a_reniced_thread_speaks_in_the_bulk_lane(self):
        seen = []

        def work():
            seen.append(self.throttle.current_priority())
            workload.renice(10)
            seen.append(self.throttle.current_priority())
            seen.append(os.nice(0))
            with workload.deezer_lane(Priority.USER):
                seen.append(self.throttle.current_priority())
            seen.append(self.throttle.current_priority())

        thread = threading.Thread(target=work)
        thread.start()
        thread.join(10)
        self.assertEqual(seen[0], self.throttle.USER)  # a request thread, by default
        self.assertEqual(seen[1], self.throttle.BULK)  # reniced: background work
        self.assertGreaterEqual(seen[2], 10)
        self.assertEqual(seen[3], self.throttle.USER)  # the queue item says why
        self.assertEqual(seen[4], self.throttle.BULK)  # and the lane goes back

    def _until(self, predicate, what, within=WAIT):
        deadline = time.monotonic() + within
        while not predicate():
            if time.monotonic() >= deadline:
                self.fail(f"timed out waiting for {what}")
            time.sleep(0.005)


class OpenStreamsTestCase(unittest.TestCase):
    """`listening_for` is a timestamp, and a one-hour episode is ONE request, so
    45 s after it started every background job came back. The open-stream count
    is what actually holds them off for the whole episode."""

    def setUp(self):
        workload.reset_streams()
        self.addCleanup(workload.reset_streams)
        workload._last_listen = 0.0
        self.addCleanup(setattr, workload, "_last_listen", 0.0)

    def test_two_streams_count_and_close(self):
        self.assertEqual(workload.streams_active(), 0)
        workload.note_stream_open()
        workload.note_stream_open()
        self.assertEqual(workload.streams_active(), 2)
        workload.note_stream_close()
        self.assertEqual(workload.streams_active(), 1)
        workload.note_stream_close()
        self.assertEqual(workload.streams_active(), 0)
        # A double close (a teardown after a generator's finally) never goes
        # negative and never makes the gate think work is running.
        workload.note_stream_close()
        self.assertEqual(workload.streams_active(), 0)

    def test_the_context_manager_closes_on_an_exception(self):
        with self.assertRaises(ZeroDivisionError):
            with workload.stream():
                self.assertEqual(workload.streams_active(), 1)
                1 / 0
        self.assertEqual(workload.streams_active(), 0)

    def test_a_background_stream_never_counts(self):
        with workload.stream(background=True):
            self.assertEqual(workload.streams_active(), 0)
        self.assertEqual(workload.streams_active(), 0)

    def test_the_throttle_waits_for_a_whole_episode(self):
        workload.note_stream_open()
        seen = []
        t = workload.Throttle(sleep=lambda _s: None, listen_quiet=45.0)

        def sleep_and_close(_seconds):
            seen.append(t.reason)
            workload.note_stream_close()

        t._sleep = sleep_and_close
        with mock.patch.object(workload, "load_per_core", lambda: 0.0):
            self.assertTrue(t.wait())
        self.assertEqual(seen, ["stream"])

    def test_a_stream_outranks_a_recent_listen_and_a_download(self):
        workload.touch_listening()
        workload.note_stream_open()
        try:
            with workload.foreground():
                t = workload.Throttle(sleep=lambda _s: None)
                self.assertEqual(t._blocked(), "stream")
        finally:
            workload.note_stream_close()


if __name__ == "__main__":
    unittest.main()
