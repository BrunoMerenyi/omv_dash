"""Container state, CPU and RAM from the Docker Engine API over its Unix socket.

Only GET requests are sent. Uses the standard library only, no Docker SDK.
"""

import http.client
import json
import os
import socket

from .. import config
from . import Background, Unavailable, safe

_previous_cpu: dict[str, tuple[int, int]] = {}  # container id -> (container cpu, system cpu)


class _UnixHTTPConnection(http.client.HTTPConnection):
    """http.client, but talking to a Unix socket instead of TCP."""

    def __init__(self, socket_path: str):
        super().__init__("localhost", timeout=5)
        self.socket_path = socket_path

    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(self.timeout)
        self.sock.connect(self.socket_path)


def _get(path: str):
    conn = _UnixHTTPConnection(config.DOCKER_SOCKET)
    try:
        conn.request("GET", path)
        response = conn.getresponse()
        body = response.read()
        if response.status != 200:
            raise Unavailable(f"Docker API returned {response.status}")
        return json.loads(body)
    finally:
        conn.close()


@safe
def _collect() -> dict:
    if not os.path.exists(config.DOCKER_SOCKET):
        raise Unavailable("Docker socket not found")
    containers = []
    for c in _get("/containers/json?all=1"):
        item = {
            "name": c["Names"][0].lstrip("/"),
            "state": c["State"],    # running, exited, restarting …
            "status": c["Status"],  # "Up 6 days (healthy)"
            "cpu": None,
            "mem": None,
        }
        if c["State"] == "running":
            # one-shot: return immediately instead of waiting for a second sample
            stats = _get(f"/containers/{c['Id']}/stats?stream=false&one-shot=true")
            item["cpu"] = _cpu_percent(c["Id"], stats)
            item["mem"] = _memory_used(stats)
        containers.append(item)
    containers.sort(key=lambda i: (i["state"] != "running", i["name"]))
    return {"containers": containers}


def _cpu_percent(container_id: str, stats: dict) -> float | None:
    """Same formula as `docker stats`, using our own previous sample."""
    cpu = stats.get("cpu_stats", {})
    total = cpu.get("cpu_usage", {}).get("total_usage")
    system = cpu.get("system_cpu_usage")
    if total is None or system is None:
        return None
    previous = _previous_cpu.get(container_id)
    _previous_cpu[container_id] = (total, system)
    if not previous or system <= previous[1]:
        return None  # first sample: no difference yet
    cpus = cpu.get("online_cpus") or 1
    return round((total - previous[0]) / (system - previous[1]) * cpus * 100, 1)


def _memory_used(stats: dict) -> int | None:
    """Usage minus page cache, like `docker stats` (cgroup v2 and v1)."""
    memory = stats.get("memory_stats", {})
    usage = memory.get("usage")
    if usage is None:
        return None
    details = memory.get("stats", {})
    return usage - details.get("inactive_file", details.get("total_inactive_file", 0))


_cache = Background(_collect, config.DOCKER_INTERVAL)


def collect() -> dict:
    return _cache.get()
