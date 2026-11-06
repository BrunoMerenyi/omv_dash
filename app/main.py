"""FastAPI app: serves the page and pushes a fresh snapshot once per second."""

import asyncio
import json
import logging
import mimetypes
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from fastapi.staticfiles import StaticFiles

from . import config, snapshot

STATIC_DIR = Path(__file__).parent / "static"
mimetypes.add_type("font/woff2", ".woff2")  # slim images have no MIME table for fonts
log = logging.getLogger("dashboard")

clients: set[WebSocket] = set()
latest: dict = {}


async def broadcast_loop():
    """Collect once per second, but only while someone is watching."""
    global latest
    loop = asyncio.get_running_loop()
    while True:
        started = loop.time()
        if clients:
            # psutil calls block, so run them in a worker thread.
            latest = await asyncio.to_thread(snapshot.build)
            message = json.dumps(latest)
            await asyncio.gather(*(send(ws, message) for ws in list(clients)))
        await asyncio.sleep(max(0.1, 1 - (loop.time() - started)))


async def send(ws: WebSocket, message: str):
    try:
        await asyncio.wait_for(ws.send_text(message), timeout=2)
    except Exception:
        clients.discard(ws)  # closed tab or very slow client


@asynccontextmanager
async def lifespan(app: FastAPI):
    log.info("OMV Dashboard on port %s (demo mode: %s)", config.PORT, config.DEMO_MODE)
    task = asyncio.create_task(broadcast_loop())
    yield
    task.cancel()


app = FastAPI(title="OMV Dashboard", lifespan=lifespan,
              docs_url=None, redoc_url=None, openapi_url=None)


@app.get("/api/snapshot")
def get_snapshot():
    """Handy for debugging: `curl http://nas:6767/api/snapshot`."""
    return latest or snapshot.build()


@app.get("/health")
def health():
    return {"ok": True}


@app.websocket("/ws")
async def websocket(ws: WebSocket):
    await ws.accept()
    clients.add(ws)
    if latest:
        await ws.send_text(json.dumps(latest))  # show something immediately
    try:
        while True:
            await ws.receive_text()  # we never expect messages; this waits for disconnect
    except WebSocketDisconnect:
        pass
    finally:
        clients.discard(ws)


@app.middleware("http")
async def no_stale_files(request, call_next):
    """Make browsers re-check files on every load (cheap 304 if unchanged),
    so a new version never mixes old CSS with new JavaScript."""
    response = await call_next(request)
    response.headers["Cache-Control"] = "no-cache"
    return response


# Must come last: serves index.html, style.css, app.js and the font.
app.mount("/", StaticFiles(directory=STATIC_DIR, html=True), name="static")


if __name__ == "__main__":
    import uvicorn

    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(name)s: %(message)s")
    uvicorn.run(app, host=config.HOST, port=config.PORT, access_log=False)
