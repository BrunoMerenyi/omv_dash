"""Active Jellyfin streams from GET /Sessions (needs an API key)."""

import json
import urllib.request

from .. import config
from . import Background, Unavailable, safe


@safe
def _collect() -> dict:
    if not (config.JELLYFIN_URL and config.JELLYFIN_API_KEY):
        raise Unavailable("JELLYFIN_URL / JELLYFIN_API_KEY not set")
    request = urllib.request.Request(
        config.JELLYFIN_URL.rstrip("/") + "/Sessions?activeWithinSeconds=120",
        headers={"Authorization": f'MediaBrowser Token="{config.JELLYFIN_API_KEY}"'},
    )
    with urllib.request.urlopen(request, timeout=5) as response:
        sessions = json.load(response)

    streams = []
    for s in sessions:
        item = s.get("NowPlayingItem")
        if not item:
            continue  # client connected but not playing
        play = s.get("PlayState") or {}
        transcoding = s.get("TranscodingInfo") or {}
        streams.append({
            "user": s.get("UserName") or "?",
            "device": s.get("DeviceName") or s.get("Client") or "?",
            "title": _title(item),
            "method": play.get("PlayMethod") or ("Transcode" if transcoding else "DirectPlay"),
            "bitrate": transcoding.get("Bitrate") or _source_bitrate(item),
            "paused": bool(play.get("IsPaused")),
        })
    return {"streams": streams}


def _title(item: dict) -> str:
    if item.get("SeriesName"):
        season = item.get("ParentIndexNumber") or 0
        episode = item.get("IndexNumber") or 0
        return f"{item['SeriesName']} · S{season:02d}E{episode:02d}"
    return item.get("Name") or "?"


def _source_bitrate(item: dict) -> int | None:
    """Bitrate of the original file when nothing is transcoded."""
    for stream in item.get("MediaStreams") or []:
        if stream.get("Type") == "Video" and stream.get("BitRate"):
            return stream["BitRate"]
    return None


_cache = Background(_collect, config.JELLYFIN_INTERVAL)


def collect() -> dict:
    return _cache.get()
