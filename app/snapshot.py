"""Builds one complete snapshot of the server: everything the page shows."""

import time

from . import config, demo


def build() -> dict:
    data = demo.build() if config.DEMO_MODE else _collect_real()
    data["temps"]["disk"] = _hottest_disk(data["disks"])
    data.update(
        title=config.TITLE or data["host"]["hostname"],
        demo=config.DEMO_MODE,
        ts=time.time(),
        limits=config.LIMITS,
        status=_overall_status(data),
    )
    return data


def _collect_real() -> dict:
    # Imported here so demo mode works even where a collector can't be imported.
    from .collectors import docker, jellyfin, minecraft, sensors, smart, storage, system

    return {
        "host": system.host(),
        "cpu": system.cpu(),
        "temps": sensors.collect(),
        "memory": system.memory(),
        "network": system.network(),
        "pool": storage.pool(),
        "disks": storage.disks(smart.latest(), sensors.disk_temps()),
        "docker": docker.collect(),
        "processes": system.processes(),
        "jellyfin": jellyfin.collect(),
        "minecraft": minecraft.collect(),
    }


def _hottest_disk(disks: dict) -> dict | None:
    with_temp = [d for d in disks.get("items", []) if d.get("temp") is not None]
    if not with_temp:
        return None
    hottest = max(with_temp, key=lambda d: d["temp"])
    return {"name": hottest["name"], "temp": hottest["temp"]}


def _overall_status(data: dict) -> dict:
    """'ok', 'warn' or 'crit', plus a short reason for every problem found."""
    reasons: list[tuple[str, str]] = []

    def check(value, limit_name, text):
        warn, crit = config.LIMITS[limit_name]
        if value is None:
            return
        if value >= crit:
            reasons.append(("crit", text))
        elif value >= warn:
            reasons.append(("warn", text))

    check(data["temps"].get("cpu"), "cpu_temp", "CPU temperature")
    check(data["memory"].get("percent"), "usage", "RAM almost full")
    check(data["pool"].get("percent"), "usage", "Pool almost full")
    for disk in data["disks"].get("items", []):
        check(disk.get("temp"), "disk_temp", f"{disk['name']} too hot")
        if disk.get("smart") is False:
            reasons.append(("crit", f"{disk['name']} SMART failed"))
    for container in data["docker"].get("containers", []):
        if "unhealthy" in (container.get("status") or ""):
            reasons.append(("warn", f"{container['name']} unhealthy"))

    levels = {level for level, _ in reasons}
    level = "crit" if "crit" in levels else "warn" if levels else "ok"
    return {"level": level, "reasons": [text for _, text in reasons]}
