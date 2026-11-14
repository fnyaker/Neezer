# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""A tiny in-process rate limiter for authentication endpoints.

Tracks failed attempts per key in a sliding window and blocks further attempts
once a threshold is reached, to slow down brute force / credential-stuffing
against an internet-exposed instance.

Two independent keys are counted for every login attempt:

* the client IP, which bounds how fast one source can guess, and
* ``user:<name>``, which bounds how fast one *account* can be guessed from
  anywhere (a botnet spread over many IPs never trips the IP key).

A successful login clears **only** the account key. Clearing the IP key too —
which is what the code used to do — handed anyone holding one valid account a
free counter reset between brute-force bursts, so the IP limit never fired.

In-process state is sufficient because the server runs as a single Gunicorn
worker (see the Dockerfile CMD); with several workers each holds its own view,
so the effective threshold is multiplied by the worker count. Behind a reverse
proxy, deploy ProxyFix (webapp/proxy_fix_hops) so request.remote_addr is the
real client IP rather than the proxy's — otherwise every client shares one key.

:class:`WindowLimiter` is the success-side twin used by the playlist routes
(``POST /api/playlists``, the membership mutations): it counts the admitted
calls themselves, answers ``(allowed, retry_after)``, and keeps its key table
bounded so a client cycling keys cannot grow it without limit.
"""

import threading
import time
from collections import OrderedDict, defaultdict, deque


class RateLimiter:
    def __init__(self, max_attempts=10, window=300):
        """max_attempts failures allowed per `window` seconds before blocking."""
        self.max_attempts = max_attempts
        self.window = window
        self._fails = defaultdict(deque)
        self._lock = threading.Lock()

    def _trim(self, dq, now):
        while dq and now - dq[0] > self.window:
            dq.popleft()

    def is_blocked(self, key):
        now = time.time()
        with self._lock:
            dq = self._fails.get(key)
            if not dq:
                return False
            self._trim(dq, now)
            if not dq:
                del self._fails[key]
                return False
            return len(dq) >= self.max_attempts

    def record_failure(self, key, username=None):
        now = time.time()
        with self._lock:
            for k in self._keys(key, username):
                dq = self._fails[k]
                self._trim(dq, now)
                dq.append(now)

    def reset(self, key):
        with self._lock:
            self._fails.pop(key, None)

    # -- composite (ip + account) keys --------------------------------------

    @staticmethod
    def user_key(username):
        return "user:" + str(username)

    def _keys(self, key, username):
        keys = [key] if key is not None else []
        if username:
            keys.append(self.user_key(username))
        return keys

    def is_blocked_any(self, key, username=None):
        """Blocked when EITHER the source IP or the target account is over."""
        return any(self.is_blocked(k) for k in self._keys(key, username))

    def reset_user(self, username):
        """Clear the account counter after a successful login.

        Deliberately does not touch the IP counter: an attacker owning one
        valid account must not be able to wipe their own guessing history.
        """
        if username:
            self.reset(self.user_key(username))


# Shared limiter for every authentication path (Subsonic API, web UI, frontend).
auth_limiter = RateLimiter()


class WindowLimiter:
    """A sliding window over the actions a person may repeat up to a cap.

    Unlike :class:`RateLimiter`, every admitted call is counted: these are
    SUCCESSES (creating a playlist, inviting somebody), not failed attempts, so
    a caller that is doing the right thing is the one that eventually trips it.
    ``allow(key)`` answers ``(allowed, retry_after_seconds)``; a refused call
    records nothing, so waiting out the window always frees exactly one slot.

    The key table is bounded (``max_keys``, least recently used evicted first):
    the keys are account ids and each call refreshes its own key, so eviction
    can never hand a caller back a counter it had just exhausted.

    In-process and lock-guarded like ``auth_limiter``: the server runs one
    Gunicorn worker (see the Dockerfile CMD), so one process holds the view.
    Tests call :meth:`clear` / :meth:`reset`.
    """

    def __init__(self, max_events, window, max_keys=4096):
        if max_events <= 0 or window <= 0:
            raise ValueError("max_events and window must be positive")
        self.max_events = max_events
        self.window = window
        self.max_keys = max(1, int(max_keys))
        self._hits = OrderedDict()
        self._lock = threading.Lock()

    def allow(self, key):
        now = time.monotonic()
        with self._lock:
            hits = self._hits.get(key)
            if hits is None:
                hits = self._hits[key] = deque()
            else:
                self._hits.move_to_end(key)
            while hits and now - hits[0] > self.window:
                hits.popleft()
            if len(hits) >= self.max_events:
                return False, max(1, int(self.window - (now - hits[0])) + 1)
            hits.append(now)
            while len(self._hits) > self.max_keys:
                self._hits.popitem(last=False)
            return True, 0

    def reset(self, key):
        with self._lock:
            self._hits.pop(key, None)

    def clear(self):
        with self._lock:
            self._hits.clear()

    def __len__(self):
        with self._lock:
            return len(self._hits)
