"""mergerfs pool usage and per-disk details.

The container needs /srv mounted read-only so it can see the pool and the
disk mounts (OMV mounts every data disk at /srv/dev-disk-by-uuid-…).
Disk list, model and size come from /sys/block; read speed from /proc/diskstats.
"""

import os

import psutil

from .. import config
from . import Rate, Unavailable, safe

_reads = Rate()
SKIP_PREFIXES = ("loop", "ram", "zram", "sr", "dm-", "md", "fd", "nbd", "mmcblk")


@safe
def pool() -> dict:
    if not os.path.isdir(config.POOL_PATH):
        raise Unavailable(f"{config.POOL_PATH} not found")
    usage = psutil.disk_usage(config.POOL_PATH)
    return {"path": config.POOL_PATH, "total": usage.total, "used": usage.used,
            "free": usage.free, "percent": usage.percent}


def physical_disks() -> list[str]:
    """Names like ["nvme0n1", "sda", "sdb"], filtered by DISKS if set."""
    names = [n for n in sorted(os.listdir("/sys/block")) if not n.startswith(SKIP_PREFIXES)]
    if config.DISKS:
        names = [n for n in names if n in config.DISKS]
    return names


@safe
def disks(smart: dict, temps: dict) -> dict:
    """`smart` and `temps` are passed in by snapshot.py: {disk name: …}."""
    io = psutil.disk_io_counters(perdisk=True) or {}
    mounts = _mount_per_disk()
    items = []
    for name in physical_disks():
        info = smart.get(name) or {}
        item = {
            "name": name,
            "model": _sys(f"/sys/block/{name}/device/model"),
            "size": int(_sys(f"/sys/block/{name}/size") or 0) * 512,  # in 512-byte sectors
            "total": None, "used": None, "percent": None,
            "temp": temps.get(name, info.get("temp")),
            "read": _reads.per_second(name, io[name].read_bytes) if name in io else None,
            "hours": info.get("hours"),
            "smart": info.get("passed"),
        }
        if name in mounts:
            usage = psutil.disk_usage(mounts[name])
            item.update(total=usage.total, used=usage.used, percent=usage.percent)
        items.append(item)
    return {"items": items}


def _mount_per_disk() -> dict[str, str]:
    """Map disk name to one mount point of its partitions, preferring /srv/…."""
    result = {}
    for part in psutil.disk_partitions():
        if not part.device.startswith("/dev/"):
            continue
        disk = _parent_disk(os.path.basename(part.device))
        if disk not in result or part.mountpoint.startswith("/srv/"):
            result[disk] = part.mountpoint
    return result


def _parent_disk(name: str) -> str:
    """"sdb1" -> "sdb", "nvme0n1p2" -> "nvme0n1" (sysfs knows the parent)."""
    path = f"/sys/class/block/{name}"
    if os.path.exists(f"{path}/partition"):
        return os.path.basename(os.path.dirname(os.path.realpath(path)))
    return name


def _sys(path: str) -> str | None:
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return None
