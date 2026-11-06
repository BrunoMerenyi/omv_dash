"""SMART health, power-on hours and temperature, refreshed every SMART_INTERVAL.

Where the data comes from (SMART_SOURCE, default "auto" picks the first that works):
  files    – <SMART_DIR>/<disk>.json, written on the host by
             `smartctl -j -a -n standby /dev/<disk>`. No device access needed
             in the container.
  smartctl – run smartctl inside the container. Only works if compose.yml
             passes the disks in and grants SYS_RAWIO (see README, SMART).

`-n standby` means sleeping disks are never woken up just for the dashboard;
their last known values are kept.
"""

import glob
import json
import os
import shutil
import subprocess

from .. import config
from . import Background, Unavailable, safe
from .storage import physical_disks

_last: dict[str, dict] = {}  # last good reading per disk (kept while a disk sleeps)


def _source(disks: list[str]) -> str:
    wanted = config.SMART_SOURCE
    if wanted in ("files", "smartctl"):
        return wanted
    if wanted == "auto":
        if glob.glob(os.path.join(config.SMART_DIR, "*.json")):
            return "files"
        if shutil.which("smartctl") and any(os.access(f"/dev/{d}", os.R_OK) for d in disks):
            return "smartctl"
    raise Unavailable("No SMART access (see README, section SMART)")


def _parse(data: dict) -> dict | None:
    if "smart_status" not in data:  # e.g. disk in standby, or no permission
        return None
    return {
        "passed": data["smart_status"].get("passed"),
        "hours": data.get("power_on_time", {}).get("hours"),
        "temp": data.get("temperature", {}).get("current"),
    }


def _read(source: str, disk: str) -> dict:
    if source == "files":
        with open(os.path.join(config.SMART_DIR, f"{disk}.json")) as f:
            return json.load(f)
    result = subprocess.run(
        ["smartctl", "-j", "-a", "-n", "standby", f"/dev/{disk}"],
        capture_output=True, text=True, timeout=30,
    )
    return json.loads(result.stdout)  # smartctl's exit code is a bit mask, so use the JSON


@safe
def _collect() -> dict:
    disks = physical_disks()
    source = _source(disks)
    for disk in disks:
        try:
            parsed = _parse(_read(source, disk))
        except (OSError, ValueError, subprocess.SubprocessError):
            parsed = None
        if parsed:
            _last[disk] = parsed
    return {"source": source, "disks": dict(_last)}


_cache = Background(_collect, config.SMART_INTERVAL)


def latest() -> dict:
    """{disk name: {"passed", "hours", "temp"}}; empty while unavailable."""
    value = _cache.get()
    return value.get("disks", {}) if value["ok"] else {}
