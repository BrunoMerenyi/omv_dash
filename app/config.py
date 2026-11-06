"""All settings come from environment variables. See .env.example for descriptions."""

import os


def _bool(name: str, default: bool = False) -> bool:
    return os.getenv(name, str(default)).strip().lower() in ("1", "true", "yes", "on")


def _int(name: str, default: int) -> int:
    try:
        return int(os.getenv(name, default))
    except ValueError:
        return default


def _list(name: str) -> list[str]:
    return [item.strip() for item in os.getenv(name, "").split(",") if item.strip()]


# General
DEMO_MODE = _bool("DEMO_MODE")
TITLE = os.getenv("DASH_TITLE", "")      # empty = the server's hostname
HOST = os.getenv("HOST", "0.0.0.0")
PORT = _int("PORT", 6767)

# System
HOST_IP = os.getenv("HOST_IP", "")        # empty = detect automatically
NET_IFACE = os.getenv("NET_IFACE", "")    # empty = busiest physical interface
TOP_PROCESSES = _int("TOP_PROCESSES", 8)

# Sensors (names as shown by `sensors` / in /sys/class/hwmon/*/name)
SENSOR_CPU_CHIP = os.getenv("SENSOR_CPU_CHIP", "k10temp")
SENSOR_BOARD_CHIP = os.getenv("SENSOR_BOARD_CHIP", "nct6798")
SENSOR_BOARD_LABEL = os.getenv("SENSOR_BOARD_LABEL", "SYSTIN")

# Storage
POOL_PATH = os.getenv("POOL_PATH", "/srv/mergerfs/pool")
DISKS = _list("DISKS")                    # empty = all physical disks

# SMART: "auto" (use whatever is reachable), "files", "smartctl" or "off"
SMART_SOURCE = os.getenv("SMART_SOURCE", "auto").lower()
SMART_DIR = os.getenv("SMART_DIR", "/smart")
SMART_INTERVAL = _int("SMART_INTERVAL", 1800)

# Docker
DOCKER_SOCKET = os.getenv("DOCKER_SOCKET", "/var/run/docker.sock")
DOCKER_INTERVAL = _int("DOCKER_INTERVAL", 5)

# Jellyfin
JELLYFIN_URL = os.getenv("JELLYFIN_URL", "")
JELLYFIN_API_KEY = os.getenv("JELLYFIN_API_KEY", "")
JELLYFIN_INTERVAL = _int("JELLYFIN_INTERVAL", 5)

# Minecraft (Java edition)
MC_HOST = os.getenv("MC_HOST", "")
MC_PORT = _int("MC_PORT", 25565)
MC_INTERVAL = _int("MC_INTERVAL", 10)

# Warning / critical thresholds (also sent to the browser so both use the same values)
LIMITS = {
    "cpu_temp": (_int("LIMIT_CPU_TEMP_WARN", 80), _int("LIMIT_CPU_TEMP_CRIT", 90)),
    "disk_temp": (_int("LIMIT_DISK_TEMP_WARN", 50), _int("LIMIT_DISK_TEMP_CRIT", 55)),
    "usage": (_int("LIMIT_USAGE_WARN", 85), _int("LIMIT_USAGE_CRIT", 95)),
}
