"""Small helpers shared by all collectors.

Every collector returns a dict with "ok": True plus its data, or
"ok": False plus an "error" text. The browser shows "Not available"
for anything that is not ok, so one broken sensor never breaks the page.
"""

import functools
import logging
import threading
import time

log = logging.getLogger("collectors")
_logged: set[tuple] = set()


class Unavailable(Exception):
    """Raise this for expected problems, e.g. 'not configured'."""


def safe(fn):
    """Decorator: turn exceptions into {"ok": False, "error": ...}."""

    @functools.wraps(fn)
    def wrapper(*args, **kwargs):
        try:
            return {"ok": True, **fn(*args, **kwargs)}
        except Exception as exc:
            error = str(exc) or type(exc).__name__
            key = (fn.__module__, fn.__name__, error)
            if key not in _logged:  # log each distinct problem only once
                _logged.add(key)
                log.warning("%s.%s: %s", fn.__module__, fn.__name__, error)
            return {"ok": False, "error": error}

    return wrapper


class Background:
    """Run a slow function in a background thread at most every `ttl` seconds.

    `get()` never blocks: it returns the last result straight away and
    starts a refresh in the background when that result is too old.
    """

    def __init__(self, fn, ttl: float):
        self.fn = fn
        self.ttl = ttl
        self.value = {"ok": False, "pending": True, "error": "Loading …"}
        self.updated = float("-inf")
        self.running = False
        self.lock = threading.Lock()

    def get(self):
        with self.lock:
            if not self.running and time.monotonic() - self.updated >= self.ttl:
                self.running = True
                threading.Thread(target=self._refresh, daemon=True).start()
        return self.value

    def _refresh(self):
        try:
            self.value = self.fn()
        finally:
            self.updated = time.monotonic()
            self.running = False


class Rate:
    """Turn ever-growing counters (bytes sent, bytes read …) into per-second rates."""

    def __init__(self):
        self.last: dict[str, tuple[float, float]] = {}

    def per_second(self, key: str, value: float) -> float:
        now = time.monotonic()
        previous = self.last.get(key)
        self.last[key] = (now, value)
        if previous is None or now <= previous[0] or value < previous[1]:
            return 0.0
        return (value - previous[1]) / (now - previous[0])
