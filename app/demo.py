"""Realistic fake data for DEMO_MODE=true, so the UI can be built on a Mac.

The shape of every section is exactly what the real collectors return.
"""

import random
import time

BOOT = time.time() - (23 * 86400 + 5 * 3600 + 17 * 60)
GB = 1000**3
TB = 1000**4
GiB = 1024**3


class Walk:
    """A value that drifts randomly between lo and hi, like a live sensor."""

    def __init__(self, value: float, lo: float, hi: float, step: float):
        self.value, self.lo, self.hi, self.step = value, lo, hi, step

    def next(self) -> float:
        self.value += random.uniform(-self.step, self.step)
        self.value = min(self.hi, max(self.lo, self.value))
        return self.value


threads = [Walk(random.uniform(3, 30), 0, 100, 9) for _ in range(16)]
cpu_temp = Walk(52, 44, 71, 0.6)
board_temp = Walk(36, 33, 40, 0.2)
fan_cpu = Walk(910, 780, 1250, 25)
fan_case = Walk(640, 560, 760, 10)
mem_used = Walk(9.8 * GiB, 8.5 * GiB, 12 * GiB, 0.05 * GiB)
net_down = Walk(4e6, 0.2e6, 42e6, 3e6)
net_up = Walk(1.8e6, 0.1e6, 9e6, 0.6e6)
stream_read = Walk(2.4e6, 0.8e6, 6e6, 0.4e6)
players = Walk(3, 0, 8, 0.15)
load = [Walk(3.2, 1.5, 9, 0.25), Walk(2.9, 1.8, 6, 0.08), Walk(2.6, 2, 4, 0.03)]

DISKS = [
    # name, model, size, used, temp, hours
    ("nvme0n1", "Samsung SSD 980 500GB", 500 * GB, 61 * GB, 41, 9120),
    ("sda", "WDC WD80EFZZ-68BTXN0", 8 * TB, 6.71 * TB, 36, 21430),
    ("sdb", "WDC WD80EFZZ-68BTXN0", 8 * TB, 6.64 * TB, 37, 21428),
    ("sdc", "ST8000VN004-3CP101", 8 * TB, 6.92 * TB, 39, 14077),
    ("sdd", "TOSHIBA MG08ADA800E", 8 * TB, 4.05 * TB, 34, 3312),
]

CONTAINERS = [
    # name, state, status, cpu walk, memory bytes
    ("jellyfin", "running", "Up 6 days", Walk(38, 12, 160, 15), 1.42 * GiB),
    ("minecraft", "running", "Up 6 days (healthy)", Walk(14, 4, 45, 4), 3.10 * GiB),
    ("immich_server", "running", "Up 6 days", Walk(2, 0.2, 18, 2), 0.88 * GiB),
    ("immich_postgres", "running", "Up 6 days", Walk(0.6, 0, 4, 0.5), 0.31 * GiB),
    ("nextcloud", "running", "Up 6 days", Walk(1, 0.1, 9, 1), 0.42 * GiB),
    ("qbittorrent", "running", "Up 2 days", Walk(4, 0.5, 22, 3), 0.27 * GiB),
    ("nas-dashboard", "running", "Up 3 hours", Walk(1.2, 0.5, 3, 0.3), 0.06 * GiB),
    ("paperless", "exited", "Exited (0) 3 days ago", None, None),
]

PROCESSES = [
    # pid, name, user, cpu walk, memory %
    (412233, "ffmpeg", "jellyfin", Walk(140, 60, 220, 18), 2.1),
    (1883, "java", "minecraft", Walk(14, 4, 45, 4), 9.6),
    (1120, "jellyfin", "jellyfin", Walk(9, 2, 25, 3), 2.4),
    (911, "mergerfs", "root", Walk(6, 1, 18, 2), 0.3),
    (2290, "qbittorrent-nox", "1000", Walk(4, 0.5, 22, 3), 0.8),
    (733, "dockerd", "root", Walk(1.5, 0.2, 6, 0.6), 0.4),
    (8814, "immich", "1000", Walk(1.2, 0, 16, 2), 2.2),
    (1301, "smbd", "root", Walk(0.8, 0, 9, 1), 0.1),
    (5120, "python3", "10001", Walk(1.1, 0.5, 3, 0.3), 0.2),
    (1, "systemd", "root", Walk(0.1, 0, 0.6, 0.1), 0.04),
]


def build() -> dict:
    thread_values = [round(t.next()) for t in threads]
    used = mem_used.next()
    disks = []
    for name, model, size, used_bytes, temp, hours in DISKS:
        busy = name == "sdc"  # the disk Jellyfin is currently reading from
        disks.append({
            "name": name, "model": model, "size": size,
            "total": size, "used": used_bytes, "percent": round(used_bytes / size * 100, 1),
            "temp": temp + (2 if busy else 0) + random.choice((0, 0, 0, 1)),
            "read": stream_read.next() if busy else random.choice((0, 0, 0, random.uniform(5e4, 4e5))),
            "hours": hours, "smart": True,
        })
    pool_disks = [d for d in disks if d["name"].startswith("sd")]
    pool_total = sum(d["total"] for d in pool_disks)
    pool_used = sum(d["used"] for d in pool_disks)

    return {
        "host": {"hostname": "openmediavault", "ip": "192.168.1.20", "uptime": int(time.time() - BOOT)},
        "cpu": {
            "ok": True,
            "percent": round(sum(thread_values) / len(thread_values), 1),
            "threads": thread_values,
            "freq_mhz": round(random.uniform(3400, 4550)),
            "model": "AMD Ryzen 7 5700G with Radeon Graphics",
            "cores": 8,
            "load": [round(w.next(), 2) for w in load],
            "core_threads": [[i, i + 8] for i in range(8)],
        },
        "temps": {
            "ok": True,
            "cpu": round(cpu_temp.next(), 1),
            "board": round(board_temp.next(), 1),
            "fans": [
                {"label": "CPU fan", "rpm": round(fan_cpu.next())},
                {"label": "Case", "rpm": round(fan_case.next())},
            ],
        },
        "memory": {
            "ok": True, "total": 32 * GiB, "used": used, "cached": 17.2 * GiB,
            "percent": round(used / (32 * GiB) * 100, 1),
            "swap_total": 4 * GiB, "swap_used": 0.13 * GiB,
        },
        "network": {"ok": True, "iface": "enp5s0", "up": net_up.next(), "down": net_down.next()},
        "pool": {
            "ok": True, "path": "/srv/mergerfs/pool", "total": pool_total, "used": pool_used,
            "free": pool_total - pool_used, "percent": round(pool_used / pool_total * 100, 1),
        },
        "disks": {"ok": True, "items": disks},
        "docker": {"ok": True, "containers": [
            {"name": n, "state": s, "status": st,
             "cpu": round(c.next(), 1) if c else None, "mem": m}
            for n, s, st, c, m in CONTAINERS
        ]},
        "processes": {"ok": True, "items": sorted(
            ({"pid": p, "name": n, "user": u, "cpu": round(c.next(), 1), "mem": m}
             for p, n, u, c, m in PROCESSES),
            key=lambda item: item["cpu"], reverse=True,
        )},
        "jellyfin": {"ok": True, "streams": [
            {"user": "anna", "title": "Severance · S02E03", "device": "Apple TV",
             "method": "DirectPlay", "bitrate": 18_400_000, "paused": False},
            {"user": "luca", "title": "Dune: Part Two", "device": "Firefox",
             "method": "Transcode", "bitrate": 8_000_000, "paused": False},
        ]},
        "minecraft": {"ok": True, "online": True, "players": round(players.next()),
                      "max": 20, "version": "1.21.4"},
    }
