"""Temperatures and fans from the Linux hwmon interface (/sys/class/hwmon).

- CPU temperature: k10temp (AMD), sensor "Tctl"/"Tdie"
- Board temperature and fan speeds: nct6798 (Nuvoton Super-I/O, needs the
  nct6775 kernel module on the host)
- Disk temperatures: the "drivetemp" (SATA) and "nvme" hwmon drivers

Reading /sys needs no special rights, so this works in an unprivileged container.
"""

import glob
import os
import re

import psutil

from .. import config
from . import safe


@safe
def collect() -> dict:
    temps = psutil.sensors_temperatures()
    fans = psutil.sensors_fans()
    return {
        "cpu": _pick(temps.get(config.SENSOR_CPU_CHIP), ("Tdie", "Tctl")),
        "board": _pick(temps.get(config.SENSOR_BOARD_CHIP), (config.SENSOR_BOARD_LABEL,)),
        "fans": [
            {"label": fan.label or f"Fan {i + 1}", "rpm": fan.current}
            for i, fan in enumerate(fans.get(config.SENSOR_BOARD_CHIP, []))
            if fan.current > 0  # unconnected headers read 0
        ],
    }


def _pick(readings, preferred_labels) -> float | None:
    """Take the reading with a preferred label, else the first one."""
    if not readings:
        return None
    for label in preferred_labels:
        for r in readings:
            if r.label == label:
                return round(r.current, 1)
    return round(readings[0].current, 1)


def disk_temps() -> dict[str, float]:
    """Disk temperatures by disk name, e.g. {"sda": 36.0, "nvme0n1": 41.0}."""
    result = {}
    for hwmon in glob.glob("/sys/class/hwmon/hwmon*"):
        try:
            name = _read(f"{hwmon}/name")
            if name == "drivetemp":     # SATA: device/block/<sdX>
                disks = os.listdir(f"{hwmon}/device/block")
            elif name == "nvme":        # NVMe: device/<nvme0n1>
                disks = [d for d in os.listdir(f"{hwmon}/device") if re.fullmatch(r"nvme\d+n\d+", d)]
            else:
                continue
            temp = int(_read(f"{hwmon}/temp1_input")) / 1000
            for disk in disks:
                result[disk] = round(temp)
        except (OSError, ValueError):
            continue
    return result


def _read(path: str) -> str:
    with open(path) as f:
        return f.read().strip()
