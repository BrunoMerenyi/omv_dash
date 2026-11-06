"""CPU, memory, network, host info and top processes, all via psutil.

Inside Docker these read the host's /proc, so they show the NAS, not the
container (network needs `network_mode: host`, processes need `pid: host`).
"""

import functools
import socket
import time

import psutil

from .. import config
from . import Rate, Unavailable, safe

_net = Rate()
SKIP_IFACES = ("lo", "docker", "br-", "veth", "virbr", "tun", "tap", "wg", "tailscale")


def host() -> dict:
    return {
        "hostname": socket.gethostname(),
        "ip": config.HOST_IP or _lan_ip(),
        "uptime": int(time.time() - psutil.boot_time()),
    }


def _lan_ip() -> str | None:
    # "Connecting" a UDP socket sends nothing, but tells us which local IP
    # the OS would use for the LAN.
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
            s.connect(("10.255.255.255", 1))
            return s.getsockname()[0]
    except OSError:
        return None


@safe
def cpu() -> dict:
    threads = psutil.cpu_percent(percpu=True)  # since the previous call
    return {
        "percent": round(sum(threads) / len(threads), 1),
        "threads": [round(t) for t in threads],
        "freq_mhz": _freq(),
        "model": _cpu_model(),
        "cores": psutil.cpu_count(logical=False),
        "load": [round(x, 2) for x in psutil.getloadavg()],  # 1, 5, 15 minutes
        "core_threads": _core_threads(len(threads)),
    }


@functools.cache
def _core_threads(count: int) -> list[list[int]]:
    """Thread numbers grouped by physical core, e.g. [[0, 8], [1, 9], …] on an 8-core Ryzen."""
    cores: dict[tuple[str, str], list[int]] = {}
    for i in range(count):
        topology = f"/sys/devices/system/cpu/cpu{i}/topology"
        try:
            with open(f"{topology}/physical_package_id") as f:
                package = f.read().strip()
            with open(f"{topology}/core_id") as f:
                core = f.read().strip()
        except OSError:
            return [[i] for i in range(count)]  # topology unknown: one thread per row
        cores.setdefault((package, core), []).append(i)
    return list(cores.values())


def _freq() -> int | None:
    try:
        freq = psutil.cpu_freq()
        return round(freq.current) if freq and freq.current else None
    except Exception:  # not supported on every platform
        return None


@functools.cache
def _cpu_model() -> str | None:
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return None


@safe
def memory() -> dict:
    vm = psutil.virtual_memory()
    swap = psutil.swap_memory()
    cached = getattr(vm, "cached", 0) + getattr(vm, "buffers", 0)  # Linux only
    return {
        "total": vm.total,
        "used": vm.used,
        "cached": cached,
        "percent": vm.percent,  # based on "available", like `free`
        "swap_total": swap.total,
        "swap_used": swap.used,
    }


@safe
def network() -> dict:
    counters = psutil.net_io_counters(pernic=True)
    iface = config.NET_IFACE or _busiest_iface(counters)
    if iface not in counters:
        raise Unavailable(f"Interface {iface or '?'} not found")
    c = counters[iface]
    return {
        "iface": iface,
        "up": _net.per_second(f"{iface}:up", c.bytes_sent),
        "down": _net.per_second(f"{iface}:down", c.bytes_recv),
    }


def _busiest_iface(counters) -> str | None:
    physical = {n: c for n, c in counters.items() if not n.startswith(SKIP_IFACES)}
    if not physical:
        return None
    return max(physical, key=lambda n: physical[n].bytes_recv + physical[n].bytes_sent)


@safe
def processes() -> dict:
    # process_iter() keeps Process objects between calls, so cpu_percent
    # measures the time since the previous call (the first call reports 0).
    items = []
    for p in psutil.process_iter(["pid", "name", "username", "cpu_percent", "memory_percent"]):
        info = p.info
        if not info["pid"]:
            continue
        items.append({
            "pid": info["pid"],
            "name": info["name"] or "?",
            "user": info["username"],
            "cpu": round(info["cpu_percent"] or 0, 1),
            "mem": round(info["memory_percent"], 1) if info["memory_percent"] is not None else None,
        })
    items.sort(key=lambda i: i["cpu"], reverse=True)
    return {"items": items[: config.TOP_PROCESSES]}
