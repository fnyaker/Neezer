"""Shared client-side rate limiter for all Deezer HTTP calls.

The public API (api.deezer.com) enforces a quota (error code 4, "Quota limit
exceeded") around ~50 requests / 5 s per IP, and the private gateway
(gw-light.php) can throttle/ban an over-eager ARL. Since both go through the
same requests.Session (same IP), a single shared limiter keeps the total
request rate to Deezer safely under the quota.

One window, several lanes. Everything the server asks Deezer goes through this
same window, so a bulk job (a discography listing one album per call, an import
paging 200 ids at a time) would otherwise put a person's search behind its
whole burst: the window is 5 s wide, so that is a tap waiting up to 5 s for
work they never asked for. A caller therefore says which lane it is in —
``USER`` for a request thread a person is waiting on (the default), ``BULK`` for
background work — and a slot that frees goes to the most urgent waiter first.
Bulk work still runs flat out when nobody is waiting; it only loses the race
while somebody is.
"""

import threading
import time
from collections import deque

USER = 0
PREFETCH = 5
BULK = 9

_local = threading.local()


def set_priority(value) -> None:
    """Declare the lane of every Deezer call this thread makes from now on."""
    _local.priority = int(value)


def current_priority() -> int:
    return int(getattr(_local, "priority", USER))


class priority:
    """``with priority(BULK)``: this thread's Deezer calls take that lane."""

    def __init__(self, value):
        self.value = int(value)
        self.previous = USER

    def __enter__(self):
        self.previous = current_priority()
        _local.priority = self.value
        return self

    def __exit__(self, *exc):
        _local.priority = self.previous
        return False


class RateLimiter:
    """Thread-safe sliding-window limiter: at most `max_calls` per `period` s.

    Waiters are served by lane (lowest number first), FIFO inside one lane. A
    lane that is held off is only ever pushed back by a lane above it, and a
    caller with nothing in front of it is admitted in the same breath — the
    ordering costs no extra latency when the window has room.
    """

    def __init__(self, max_calls: int, period: float):
        self.max_calls = max_calls
        self.period = period
        self._calls = deque()
        self._waiting = {}
        self._cond = threading.Condition(threading.Lock())

    def _expire(self, now):
        while self._calls and now - self._calls[0] >= self.period:
            self._calls.popleft()

    def _ahead(self, level):
        return any(n > 0 and p < level for p, n in self._waiting.items())

    def _delay(self, now):
        if self._calls:
            return max(0.001, self.period - (now - self._calls[0]))
        return 0.05

    def acquire(self, level=None) -> None:
        """Take one slot, waiting in the lane ``level`` (this thread's by default)."""
        lane = current_priority() if level is None else int(level)
        with self._cond:
            self._waiting[lane] = self._waiting.get(lane, 0) + 1
            try:
                while True:
                    now = time.monotonic()
                    self._expire(now)
                    if len(self._calls) < self.max_calls and not self._ahead(lane):
                        self._calls.append(now)
                        self._cond.notify_all()
                        return
                    self._cond.wait(self._delay(now))
            finally:
                left = self._waiting.get(lane, 1) - 1
                if left > 0:
                    self._waiting[lane] = left
                else:
                    self._waiting.pop(lane, None)
                self._cond.notify_all()

    def waiting(self, level=None) -> int:
        """How many callers are waiting in a lane (or in all of them)."""
        with self._cond:
            if level is None:
                return sum(self._waiting.values())
            return int(self._waiting.get(int(level), 0))

    def clear(self) -> None:
        with self._cond:
            self._calls.clear()
            self._cond.notify_all()


# Conservative shared default (~8 req/s), well under the public ~50/5s quota.
limiter = RateLimiter(max_calls=40, period=5.0)
