# OMV Dashboard

A small live dashboard for a Debian/OpenMediaVault NAS. FastAPI pushes one
snapshot per second over a WebSocket; the page is plain HTML, CSS and JS.

```
app/
  main.py            routes + WebSocket (one broadcast loop for all browsers)
  snapshot.py        assembles all collectors + overall status
  config.py          every setting, read from environment variables
  demo.py            fake data for DEMO_MODE=true
  collectors/
    __init__.py      helpers: safe(), Background cache, Rate
    system.py        CPU, RAM, network, IP, uptime, processes (psutil)
    sensors.py       k10temp, nct6798, fan RPM, disk temps (hwmon)
    storage.py       mergerfs pool, per-disk usage and read speed
    smart.py         SMART health / hours (cached, 30 min)
    docker.py        container state, CPU, RAM (Docker socket)
    jellyfin.py      active streams (Jellyfin API)
    minecraft.py     online + players (Server List Ping)
  static/            index.html, style.css, app.js, fonts/
tests/               pytest
```

**How it works:** each collector returns `{"ok": true, ...}` or
`{"ok": false, "error": "..."}`. The page shows **"Not available"** for
anything that isn't ok, so a missing sensor never breaks the page.
Slow collectors (Docker, Jellyfin, Minecraft, SMART) run in a background
thread at their own interval and never slow down the 1-second loop.
Graph history (2 minutes) is kept in the browser.

## 1. On the Mac: develop with demo data

Needs [uv](https://docs.astral.sh/uv/) (or any Python 3.12).

```bash
uv venv --python 3.12 .venv
```

```bash
uv pip install --python .venv -r requirements-dev.txt
```

```bash
DEMO_MODE=true .venv/bin/python -m app.main
```

Open http://localhost:6767. Edit files in `app/static/` and reload the browser.

Run the tests:

```bash
.venv/bin/python -m pytest -q
```

The real collectors are written for Linux. On the Mac, use demo mode.
To run the tests against real Linux data, use the container (step 2).

## 2. Build the image for the NAS (linux/amd64)

Docker Desktop must be running.

```bash
docker buildx build --platform linux/amd64 -t nas-dashboard:latest --load .
```

Optional: try it on the Mac in demo mode (runs under emulation):

```bash
docker run --rm --platform linux/amd64 -p 6767:6767 -e DEMO_MODE=true nas-dashboard:latest
```

## 3. Transfer to the NAS

Replace `root@nas` with your SSH login. This streams the image straight
into Docker on the NAS without a registry:

```bash
docker save nas-dashboard:latest | gzip | ssh root@nas 'gunzip | docker load'
```

Copy the compose file and the example config (first time only):

```bash
ssh root@nas 'mkdir -p /opt/nas-dashboard'
```

```bash
scp compose.yml .env.example root@nas:/opt/nas-dashboard/
```

## 4. Run on the NAS

```bash
cd /opt/nas-dashboard && cp .env.example .env && chmod 600 .env
```

Set `DOCKER_GID` (output of `getent group docker | cut -d: -f3`), the
Jellyfin API key and anything else in `.env`, then:

```bash
docker compose up -d
```

Open `http://<nas-ip>:6767`. After a new build + `docker load`, run
`docker compose up -d` again to restart with the new image.

Check what the collectors see: `curl -s localhost:6767/api/snapshot | python3 -m json.tool`

## Access the container gets, and why

| Setting | Why | Risk |
|---|---|---|
| `network_mode: host` | real NIC counters + LAN IP; reach Jellyfin/Minecraft on 127.0.0.1 | port 6767 opens directly on the NAS |
| `pid: host` | "Processes" card | container can *see* host processes, not touch them |
| `/var/run/docker.sock:ro` | container list + stats | see below |
| `/srv:/srv:ro,rslave` | pool + per-disk usage | the unprivileged user can read world-readable files under /srv |
| `cap_drop: ALL`, `no-new-privileges`, `read_only`, uid 10001 | hardening | – |

**Docker socket:** `:ro` only stops the socket *file* being replaced. Anyone
who can talk to the socket can use the whole Docker API, which is
equivalent to root on the host. This app only sends `GET` requests. If you
want that enforced, put a socket proxy in between (e.g.
`tecnativa/docker-socket-proxy` with only `CONTAINERS=1`).

**LAN only:** there is no authentication, so don't forward port 6767 on
your router or put it behind a public reverse proxy. To bind to the LAN
interface only (and not e.g. a VPN interface), set `HOST=<lan ip>` in `.env`.

## Sensors

- **CPU** (`k10temp`) and **disk temps** (`drivetemp` for SATA, `nvme`)
  come from `/sys/class/hwmon` and need no extra rights.
- **Board temperature and fans** need the `nct6775` kernel module on the
  host. Check with `sensors`; if `nct6798` is missing, run
  `modprobe nct6775` and add `nct6775` to `/etc/modules`. Some boards also
  need `acpi_enforce_resources=lax` on the kernel command line.
- **SATA disk temperatures without SMART:** `modprobe drivetemp` and add
  `drivetemp` to `/etc/modules`.

Names differ? Run `ls /sys/class/hwmon/*/name; sensors` and set
`SENSOR_CPU_CHIP`, `SENSOR_BOARD_CHIP`, `SENSOR_BOARD_LABEL`.

## SMART

SMART (health, power-on hours) needs raw access to the disks. A container
can't give itself that access; it has to be allowed from the outside. With
`SMART_SOURCE=auto` (default) the dashboard uses whichever of these it
finds, or shows "Not available" if there is neither.

### Option A (recommended): the host writes JSON, the container reads it

The container gets **no** device access. A systemd timer on the NAS runs
`smartctl` every 30 minutes, as root, and writes one JSON file per disk.
`-n standby` means sleeping disks are not woken up.

On the NAS, as root:

```bash
mkdir -p /var/lib/nas-dashboard/smart
cat > /etc/systemd/system/nas-dashboard-smart.service <<'EOF'
[Unit]
Description=Write SMART data for nas-dashboard

[Service]
Type=oneshot
ExecStart=/bin/sh -c 'for d in /sys/block/sd* /sys/block/nvme*n*; do [ -e "$d" ] || continue; n=$(basename "$d"); smartctl -j -a -n standby "/dev/$n" > "/var/lib/nas-dashboard/smart/$n.json.tmp"; mv "/var/lib/nas-dashboard/smart/$n.json.tmp" "/var/lib/nas-dashboard/smart/$n.json"; done'
EOF
cat > /etc/systemd/system/nas-dashboard-smart.timer <<'EOF'
[Unit]
Description=Write SMART data for nas-dashboard every 30 minutes

[Timer]
OnBootSec=2min
OnUnitActiveSec=30min

[Install]
WantedBy=timers.target
EOF
systemctl daemon-reload && systemctl enable --now nas-dashboard-smart.timer
```

`compose.yml` already mounts `/var/lib/nas-dashboard/smart` read-only, so
SMART data appears on the dashboard within the next refresh (30 min, or
restart the container to pick it up immediately).

### Option B: run smartctl inside the container

Simpler on the host, but the container needs a lot more:

- **`devices:`** every disk, e.g. `/dev/sda`, … The container can then
  read and **write** those raw disks.
- **`cap_add: [SYS_RAWIO]`** (SATA). NVMe also needs `SYS_ADMIN`.
- **`user: root`**: capabilities only reach root processes, unless the image
  is changed to give `smartctl` file capabilities.

Option A gives the same data with far less access, so this project doesn't
enable option B. If you want it anyway, add the three settings above to
`compose.yml`.
