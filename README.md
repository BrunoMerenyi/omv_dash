# OMV Dashboard

A live dashboard for an OpenMediaVault (Debian) NAS: CPU, temperatures, RAM,
network, mergerfs pool and disks, Docker containers, top processes, Jellyfin
streams and Minecraft players, updated every second.

- **Tiny:** one 0.65 MB binary, about 1 MB of RAM, under 0.5 % CPU.
- **Read-only:** it can look at your server, never change it.
- **No setup screens:** everything is detected automatically.

---

## Install

**1. Find your Docker group number** (on the NAS):

```bash
getent group docker | cut -d: -f3
```

**2. Add this compose file.** In OMV: *Services → Compose → Files → Add*.
Without OMV: save it as `compose.yml` in a new folder.
Replace `999` with the number from step 1:

```yaml
services:
  nas-dashboard:
    image: ghcr.io/brunomerenyi/omv_dash:latest
    pull_policy: always
    container_name: nas-dashboard
    network_mode: host
    pid: host
    environment:
      JELLYFIN_API_KEY: ""      # optional: Jellyfin → Dashboard → API Keys
      MC_HOST: ""               # optional: Minecraft Java server, e.g. 127.0.0.1
    read_only: true
    cap_drop: [ALL]
    security_opt: [no-new-privileges:true]
    volumes:
      - /srv:/srv:ro,rslave
      - /var/lib/nas-dashboard/smart:/smart:ro
    restart: unless-stopped

  docker-proxy:
    image: wollomatic/socket-proxy:1
    container_name: docker-proxy
    network_mode: host
    command: [-loglevel=WARN, '-allowGET=^/containers/(json|[0-9a-f]{64}/stats)$$']
    user: "65534:999"           # ← your number from step 1
    environment: [GOMEMLIMIT=4MiB]
    read_only: true
    cap_drop: [ALL]
    security_opt: [no-new-privileges:true]
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
    restart: unless-stopped
```

**Already have a compose file** (e.g. for Jellyfin and friends)? Copy only the
two services, everything from `nas-dashboard:` to the end, and paste them under
your existing `services:` line. Keep the two-space indent and skip the first
`services:` line.

**3. Start it.** In OMV: select the file and click *Up*. Without OMV:

```bash
docker compose up -d
```

Your other containers keep running. Compose only starts what's new or changed.

**4. Open** `http://<your-nas-ip>:6767`

Anything the dashboard can't read shows **"Not available"** with a short
reason. The optional steps below fix the common ones.

---

## Optional: SMART data (disk health, power-on hours)

The dashboard never gets access to your disks. Instead, a small timer on the
NAS saves the SMART data every 30 minutes, and the dashboard reads those
files. Sleeping disks are not woken up. Run this once, as root:

```bash
mkdir -p /var/lib/nas-dashboard/smart
cat > /etc/systemd/system/nas-dashboard-smart.service <<'EOF'
[Unit]
Description=Save SMART data for nas-dashboard

[Service]
Type=oneshot
ExecStart=/bin/sh -c 'for d in /sys/block/sd* /sys/block/nvme*n*; do [ -e "$d" ] || continue; n=$(basename "$d"); smartctl -j -a -n standby "/dev/$n" > "/var/lib/nas-dashboard/smart/$n.json.tmp"; mv "/var/lib/nas-dashboard/smart/$n.json.tmp" "/var/lib/nas-dashboard/smart/$n.json"; done'
EOF
cat > /etc/systemd/system/nas-dashboard-smart.timer <<'EOF'
[Unit]
Description=Save SMART data for nas-dashboard every 30 minutes

[Timer]
OnBootSec=2min
OnUnitActiveSec=30min

[Install]
WantedBy=timers.target
EOF
systemctl daemon-reload && systemctl enable --now nas-dashboard-smart.timer
```

## Optional: board temperature and fan speeds

These come from the mainboard's sensor chip. If they show "Not available":

```bash
modprobe nct6775 && echo nct6775 >> /etc/modules
```

For SATA disk temperatures without SMART: `modprobe drivetemp && echo drivetemp >> /etc/modules`

---

## Update

Run `docker compose up -d` again (in OMV: *Up* again). It always fetches the
newest version.

## Settings

All optional. Add them under `environment:` of `nas-dashboard`:

| Setting | Default | What it does |
|---|---|---|
| `JELLYFIN_API_KEY` | – | Shows Jellyfin streams |
| `JELLYFIN_URL` | `http://127.0.0.1:8096` | Where Jellyfin runs |
| `MC_HOST` / `MC_PORT` | – / `25565` | Minecraft Java server |
| `PORT` | `6767` | Port of the dashboard |
| `DASH_TITLE` | hostname | Title in the header |
| `POOL_PATH` | `/srv/mergerfs/pool` | Your mergerfs pool |
| `DISKS` | all | Only show these disks, e.g. `sda,sdb` |
| `NET_IFACE` | busiest | Network interface to show |
| `PROCESS_INTERVAL` | `3` | Seconds between process list updates |
| `DOCKER_INTERVAL` | `10` | Seconds between container updates |
| `SENSOR_CPU_CHIP` / `SENSOR_BOARD_CHIP` | `k10temp` / `nct6798` | Sensor chips (see `sensors`) |
| `LIMIT_CPU_TEMP_WARN` / `_CRIT` | `80` / `90` | When the status turns yellow / red |
| `LIMIT_DISK_TEMP_WARN` / `_CRIT` | `50` / `55` | Same for disks |
| `LIMIT_USAGE_WARN` / `_CRIT` | `85` / `95` | Same for RAM and pool usage (%) |

## Security

- **No login, so keep it on your LAN.** Don't forward port 6767 or put it
  behind a public proxy. Access over Tailscale/VPN is fine.
- **Read-only:** the page has no buttons. The server only answers `GET`
  requests, and every mount is read-only.
- **No Docker socket for the dashboard.** The small `docker-proxy` container
  lets exactly two requests through ("list containers" and "container stats")
  and refuses everything else, including reading container settings, which
  could contain passwords.
- **Locked down:** the container runs as an unprivileged user, with no
  capabilities and a read-only filesystem. The image contains nothing but the
  binary.

---

## Development

Written in Rust, with a plain HTML/CSS/JS page compiled into the binary.

```bash
DEMO_MODE=true cargo run        # fake data, http://localhost:6767
cargo test
```

Build and publish the image for the NAS (builds on any Mac or PC, for amd64):

```bash
docker buildx build --platform linux/amd64 -t ghcr.io/brunomerenyi/omv_dash:2.0.0 -t ghcr.io/brunomerenyi/omv_dash:latest --push .
```
