"""Run with: pytest -q   (real-mode tests are meant to run on Linux, e.g. in the container)"""

import json
import os
import socket
import socketserver
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer

import pytest
from fastapi.testclient import TestClient

from app import config, snapshot
from app.collectors import docker, jellyfin, minecraft, smart

SECTIONS = ["cpu", "temps", "memory", "network", "pool", "disks",
            "docker", "processes", "jellyfin", "minecraft"]


# ---------- Snapshot & WebSocket ----------

def test_demo_snapshot(monkeypatch):
    monkeypatch.setattr(config, "DEMO_MODE", True)
    monkeypatch.setattr(config, "TITLE", "")
    snap = snapshot.build()
    json.dumps(snap)  # must be serialisable
    assert all(snap[s]["ok"] for s in SECTIONS)
    assert snap["status"]["level"] in ("ok", "warn", "crit")
    assert snap["title"] == "openmediavault"  # hostname, since DASH_TITLE is not set


@pytest.mark.skipif(not sys.platform.startswith("linux"), reason="real collectors target Linux")
def test_real_snapshot_never_crashes(monkeypatch):
    monkeypatch.setattr(config, "DEMO_MODE", False)
    snap = snapshot.build()
    json.dumps(snap)
    for section in SECTIONS:
        assert "ok" in snap[section], section
    assert snap["cpu"]["ok"] and snap["memory"]["ok"]


def test_websocket_pushes_snapshots(monkeypatch):
    monkeypatch.setattr(config, "DEMO_MODE", True)
    from app.main import app

    with TestClient(app) as client:
        assert client.get("/").status_code == 200
        with client.websocket_connect("/ws") as ws:
            assert "cpu" in ws.receive_json()


# ---------- Minecraft ----------

class FakeMinecraft(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.recv(1024)  # handshake + status request
        body = json.dumps({"version": {"name": "1.21.4"},
                           "players": {"online": 3, "max": 20}}).encode()
        payload = minecraft._varint(0) + minecraft._varint(len(body)) + body
        self.request.sendall(minecraft._varint(len(payload)) + payload)


def test_minecraft_online_and_offline(monkeypatch):
    server = socketserver.TCPServer(("127.0.0.1", 0), FakeMinecraft)
    threading.Thread(target=server.handle_request, daemon=True).start()
    monkeypatch.setattr(config, "MC_HOST", "127.0.0.1")
    monkeypatch.setattr(config, "MC_PORT", server.server_address[1])
    result = minecraft._collect()
    server.server_close()
    assert result == {"ok": True, "online": True, "players": 3, "max": 20, "version": "1.21.4"}

    # Nobody listening any more -> offline, not an error
    assert minecraft._collect()["online"] is False


def test_minecraft_not_configured(monkeypatch):
    monkeypatch.setattr(config, "MC_HOST", "")
    assert minecraft._collect()["ok"] is False


# ---------- Jellyfin ----------

SESSIONS = [
    {"UserName": "anna", "DeviceName": "Apple TV",
     "NowPlayingItem": {"SeriesName": "Severance", "ParentIndexNumber": 2, "IndexNumber": 3,
                        "MediaStreams": [{"Type": "Video", "BitRate": 18_000_000}]},
     "PlayState": {"PlayMethod": "DirectPlay", "IsPaused": False}},
    {"UserName": "luca", "DeviceName": "Firefox",
     "NowPlayingItem": {"Name": "Dune"},
     "PlayState": {"PlayMethod": "Transcode"}, "TranscodingInfo": {"Bitrate": 8_000_000}},
    {"UserName": "idle", "DeviceName": "Phone"},  # connected, not playing
]


class FakeJellyfin(BaseHTTPRequestHandler):
    def do_GET(self):
        ok = self.headers.get("Authorization") == 'MediaBrowser Token="secret"'
        self.send_response(200 if ok else 401)
        self.end_headers()
        if ok:
            self.wfile.write(json.dumps(SESSIONS).encode())

    def log_message(self, *args):
        pass


def test_jellyfin_streams(monkeypatch):
    server = HTTPServer(("127.0.0.1", 0), FakeJellyfin)
    threading.Thread(target=server.handle_request, daemon=True).start()
    monkeypatch.setattr(config, "JELLYFIN_URL", f"http://127.0.0.1:{server.server_address[1]}")
    monkeypatch.setattr(config, "JELLYFIN_API_KEY", "secret")
    result = jellyfin._collect()
    server.server_close()
    assert result["ok"]
    assert [s["title"] for s in result["streams"]] == ["Severance · S02E03", "Dune"]
    assert [s["method"] for s in result["streams"]] == ["DirectPlay", "Transcode"]
    assert [s["bitrate"] for s in result["streams"]] == [18_000_000, 8_000_000]


# ---------- Docker ----------

CONTAINERS = [
    {"Id": "a1", "Names": ["/jellyfin"], "State": "running", "Status": "Up 2 days"},
    {"Id": "b2", "Names": ["/paperless"], "State": "exited", "Status": "Exited (0)"},
]


class FakeDocker(BaseHTTPRequestHandler):
    calls = 0

    def do_GET(self):
        if self.path.startswith("/containers/json"):
            body = CONTAINERS
        else:
            FakeDocker.calls += 1
            n = FakeDocker.calls
            body = {"cpu_stats": {"cpu_usage": {"total_usage": n * 500}, "system_cpu_usage": n * 1000,
                                  "online_cpus": 4},
                    "memory_stats": {"usage": 300, "stats": {"inactive_file": 100}}}
        self.send_response(200)
        self.end_headers()
        self.wfile.write(json.dumps(body).encode())

    def address_string(self):
        return "unix"

    def log_message(self, *args):
        pass


def test_docker_over_unix_socket(monkeypatch):
    path = os.path.join(tempfile.mkdtemp(), "docker.sock")
    server = socketserver.UnixStreamServer(path, FakeDocker)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    monkeypatch.setattr(config, "DOCKER_SOCKET", path)
    docker._collect()                 # first sample: no CPU % yet
    result = docker._collect()
    server.shutdown()
    jf, paperless = result["containers"]
    assert jf == {"name": "jellyfin", "state": "running", "status": "Up 2 days", "cpu": 200.0, "mem": 200}
    assert paperless["cpu"] is None


def test_docker_socket_missing(monkeypatch):
    monkeypatch.setattr(config, "DOCKER_SOCKET", "/nonexistent.sock")
    assert docker._collect() == {"ok": False, "error": "Docker socket not found"}


# ---------- SMART ----------

def test_smart_from_files(monkeypatch, tmp_path):
    (tmp_path / "sda.json").write_text(json.dumps({
        "smart_status": {"passed": True}, "power_on_time": {"hours": 21430},
        "temperature": {"current": 36}}))
    (tmp_path / "sdb.json").write_text(json.dumps({"smartctl": {"exit_status": 2}}))  # in standby
    monkeypatch.setattr(config, "SMART_SOURCE", "auto")  # should find the files by itself
    monkeypatch.setattr(config, "SMART_DIR", str(tmp_path))
    monkeypatch.setattr(smart, "physical_disks", lambda: ["sda", "sdb"])
    smart._last.clear()
    result = smart._collect()
    assert result["source"] == "files"
    assert result["disks"] == {"sda": {"passed": True, "hours": 21430, "temp": 36}}


def test_smart_without_access(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "SMART_SOURCE", "auto")
    monkeypatch.setattr(config, "SMART_DIR", str(tmp_path))      # empty
    monkeypatch.setattr(smart, "physical_disks", lambda: ["sdzz"])  # no such device
    assert smart._collect()["ok"] is False
