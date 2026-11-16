"""A bench leaves the machine as it found it: no synthetic library behind it.

``tools/perf_api.py`` builds its library in ``$TMPDIR/nsperf-*`` — 5 MB for a
small seed, 2.3 GB for the 20 000-track one — and on most machines ``/tmp`` is a
tmpfs, so what it leaks is RAM. On 2026-11-16 hundreds of leftovers filled the
shared machine's tmpfs (19 GB of 24, the user quota reached, EDQUOT for
everyone), because the tool only removed the directory when it reached the end
of its own code: the browser benches stop it with SIGTERM, so the end never came.

So the tool removes its library on every exit it can catch (``atexit`` for a
return, an exception and ``sys.exit``; signal handlers for SIGTERM, SIGINT and
SIGHUP), writes an ``owner`` file naming the pid that owns the directory, and
sweeps at startup what a SIGKILL left behind — never touching a directory whose
owner is still alive, because several workers run benches at once.

These tests own their temporary directory (``TMPDIR`` on the child, ``root=`` on
the sweep) and never look at the machine's, where other agents' benches live.
"""

import importlib.util
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from functools import lru_cache

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TOOL = os.path.join(ROOT, "tools", "perf_api.py")
# One track and one playlist: what is measured here is the directory's fate,
# not the library's size. A launch takes ~1 s (measured).
SEED = [
    "--tracks", "1", "--playlists", "1", "--favorites", "1",
    "--big-playlist", "1", "--runs", "1", "--port", "0",
]
STARTUP_TIMEOUT = 180


@lru_cache(maxsize=1)
def load_tool():
    """``tools/perf_api.py`` as a module: its clean-up helpers, without running it."""
    spec = importlib.util.spec_from_file_location("neezer_perf_api", TOOL)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PerfLibraryTestCase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="nsperf-test-")
        self.children = []

    def tearDown(self):
        for child in self.children:
            if child.poll() is None:
                child.kill()
            child.wait(timeout=30)
        shutil.rmtree(self.tmp, ignore_errors=True)

    # -- the bench itself --------------------------------------------------

    def libraries(self):
        return sorted(
            name for name in os.listdir(self.tmp) if name.startswith("nsperf-")
        )

    def launch(self, *extra):
        """Start the tool on this test's TMPDIR; return (process, work dir)."""
        process = subprocess.Popen(
            [sys.executable, TOOL, *SEED, *extra],
            cwd=ROOT,
            env=dict(os.environ, TMPDIR=self.tmp),
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )
        self.children.append(process)
        watchdog = threading.Timer(STARTUP_TIMEOUT, process.kill)
        watchdog.daemon = True
        watchdog.start()
        try:
            for line in process.stdout:
                if not line.startswith("READY "):
                    continue
                process.stdout.close()
                owner = os.path.join(self.tmp, self.libraries()[0], "owner")
                with open(owner, encoding="utf-8") as handle:
                    self.assertEqual(int(handle.read().split()[0]), process.pid)
                return process
        finally:
            watchdog.cancel()
        self.fail(f"perf_api.py exited ({process.poll()}) before READY")

    def stop(self, process, sig=signal.SIGTERM):
        process.send_signal(sig)
        return process.wait(timeout=120)

    def test_sigterm_removes_the_library(self):
        """The browser benches' way out: SIGTERM, then nothing on disk.

        Measured: 20 of 20 launches left 0 directories, exit status -15.
        """
        process = self.launch("--serve")
        self.assertEqual(len(self.libraries()), 1, "the bench built its library")
        code = self.stop(process, signal.SIGTERM)
        self.assertEqual(code, -signal.SIGTERM, "the SIGTERM that stops a bench appears in its status")
        self.assertEqual(self.libraries(), [], "SIGTERM left a library behind")

    def test_normal_end_removes_the_library(self):
        """`python tools/perf_api.py` with no --serve: the atexit path."""
        process = subprocess.Popen(
            [sys.executable, TOOL, "--tracks", "1", "--playlists", "1", "--favorites", "1",
             "--big-playlist", "1", "--runs", "1"],
            cwd=ROOT,
            env=dict(os.environ, TMPDIR=self.tmp),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        self.children.append(process)
        self.assertEqual(process.wait(timeout=STARTUP_TIMEOUT), 0)
        self.assertEqual(self.libraries(), [], "a bench that ended normally left its library")

    def test_sigkill_orphan_is_swept_by_the_next_launch(self):
        """SIGKILL cannot be caught: the next bench collects what it left.

        Measured: 20 of 20 SIGKILL launches left exactly 1 directory, and the
        launch after each left 0.
        """
        killed = self.launch("--serve")
        self.stop(killed, signal.SIGKILL)
        left = self.libraries()
        self.assertEqual(len(left), 1, "a SIGKILLed bench cannot clean up after itself")

        nxt = self.launch("--serve")
        self.assertEqual(len(self.libraries()), 1, "the sweep left the new bench's own library alone")
        self.assertNotEqual(self.libraries()[0], left[0], "the orphan is gone, the newcomer is there")
        self.stop(nxt)
        self.assertEqual(self.libraries(), [])

    # -- the sweep's two safety rules -------------------------------------

    def test_sweep_keeps_a_library_whose_owner_is_alive(self):
        """Several workers run benches at once: a live owner is never swept."""
        perf = load_tool()
        live = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(120)"])
        self.children.append(live)
        # A pid that has been reaped: the number is free, as a killed bench's is.
        reaped = subprocess.Popen([sys.executable, "-c", "pass"])
        reaped.wait(timeout=30)
        self.assertFalse(perf._pid_alive(reaped.pid), "a reaped pid counts as dead")
        alive = os.path.join(self.tmp, "nsperf-alive")
        dead = os.path.join(self.tmp, "nsperf-dead")
        for work, pid in ((alive, live.pid), (dead, reaped.pid)):
            os.makedirs(work)
            with open(os.path.join(work, "owner"), "w", encoding="utf-8") as handle:
                handle.write(f"{pid}\n")
        self.assertEqual(perf.sweep_orphans(self.tmp), [dead])
        self.assertTrue(os.path.isdir(alive), "a living bench lost its library")

    def test_sweep_keeps_an_unowned_library_a_live_process_holds_open(self):
        """The directories the tools that predate the owner file wrote.

        A serving bench keeps its SQLite database open (4 fds measured on an
        idle one: db, -shm, -wal), which is what tells it from the directories a
        killed bench left.
        """
        perf = load_tool()
        work = os.path.join(self.tmp, "nsperf-held")
        os.makedirs(work)
        held = os.path.join(work, "db.sqlite")
        open(held, "w", encoding="utf-8").close()
        holder = subprocess.Popen(
            [
                sys.executable,
                "-c",
                "import sys, time; held = open(sys.argv[1]); time.sleep(120)",
                held,
            ]
        )
        self.children.append(holder)
        self.assertTrue(self.wait_held_by(holder.pid, held), "the holder never opened the database")
        # Old enough to be judged: the 60 s grace is for a peer between its
        # mkdtemp and its own owner file, not for a bench that is running.
        old = time.time() - 3600
        os.utime(work, (old, old))
        self.assertEqual(perf.sweep_orphans(self.tmp), [], "a held library was swept")
        self.assertTrue(os.path.isdir(work))

        self.stop(holder, signal.SIGKILL)
        self.assertEqual(perf.sweep_orphans(self.tmp), [work], "nobody holds it: it is an orphan")

    @staticmethod
    def wait_held_by(pid, path):
        """Wait for `pid` to really have `path` open before judging anything."""
        deadline = time.time() + 30
        fds = f"/proc/{pid}/fd"
        while time.time() < deadline:
            for entry in os.listdir(fds) if os.path.isdir(fds) else []:
                try:
                    if os.readlink(os.path.join(fds, entry)) == path:
                        return True
                except OSError:
                    continue
            time.sleep(0.01)
        return False

    def test_sweep_leaves_a_young_unowned_library_alone(self):
        """A peer between `mkdtemp` and its owner file looks unowned for an instant."""
        perf = load_tool()
        work = os.path.join(self.tmp, "nsperf-young")
        os.makedirs(work)
        self.assertEqual(perf.sweep_orphans(self.tmp), [])
        self.assertTrue(os.path.isdir(work))

    def test_sweep_ignores_everything_that_is_not_a_library(self):
        perf = load_tool()
        os.makedirs(os.path.join(self.tmp, "other-tool"))
        self.assertEqual(perf.sweep_orphans(self.tmp), [])
        self.assertTrue(os.path.isdir(os.path.join(self.tmp, "other-tool")))


if __name__ == "__main__":
    unittest.main()
