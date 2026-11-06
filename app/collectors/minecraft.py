"""Minecraft (Java edition) online state and player count.

Uses the "Server List Ping" that the game's server browser uses:
https://minecraft.wiki/w/Java_Edition_protocol/Server_List_Ping
No RCON, no plugins, no passwords needed.
"""

import json
import socket
import struct

from .. import config
from . import Background, Unavailable, safe


def _varint(value: int) -> bytes:
    """Minecraft's variable-length integer: 7 bits per byte, high bit = 'more follows'."""
    value &= 0xFFFFFFFF
    out = b""
    while True:
        byte = value & 0x7F
        value >>= 7
        out += bytes([byte | (0x80 if value else 0)])
        if not value:
            return out


def _read_varint(sock: socket.socket) -> int:
    value = shift = 0
    while True:
        byte = _recv_exact(sock, 1)[0]
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return value
        shift += 7
        if shift > 35:
            raise ValueError("VarInt too long")


def _recv_exact(sock: socket.socket, size: int) -> bytes:
    data = b""
    while len(data) < size:
        chunk = sock.recv(size - len(data))
        if not chunk:
            raise ConnectionError("Connection closed")
        data += chunk
    return data


def _packet(packet_id: int, payload: bytes = b"") -> bytes:
    body = _varint(packet_id) + payload
    return _varint(len(body)) + body


def ping(host: str, port: int, timeout: float = 3) -> dict:
    """Return the server's status JSON (version, players, description)."""
    with socket.create_connection((host, port), timeout=timeout) as sock:
        address = host.encode()
        handshake = (_varint(-1)                          # protocol version: "unknown"
                     + _varint(len(address)) + address
                     + struct.pack(">H", port)
                     + _varint(1))                        # next state: status
        sock.sendall(_packet(0x00, handshake) + _packet(0x00))
        _read_varint(sock)                                # packet length
        _read_varint(sock)                                # packet id
        length = _read_varint(sock)
        return json.loads(_recv_exact(sock, length))


@safe
def _collect() -> dict:
    if not config.MC_HOST:
        raise Unavailable("MC_HOST not set")
    try:
        status = ping(config.MC_HOST, config.MC_PORT)
    except (OSError, ValueError):
        return {"online": False, "players": 0, "max": None, "version": None}
    players = status.get("players", {})
    return {
        "online": True,
        "players": players.get("online", 0),
        "max": players.get("max"),
        "version": status.get("version", {}).get("name"),
    }


_cache = Background(_collect, config.MC_INTERVAL)


def collect() -> dict:
    return _cache.get()
