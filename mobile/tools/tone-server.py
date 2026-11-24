#!/usr/bin/env python3
"""The tone server `NzMediaServiceTest` plays against (MB3c's fixture, in-tree).

    mobile/tools/tone-server.py [--port 4711] [--flac <path>]

It answers the three things the Rust player asks of a server for a queue of
`TrackRef`s that are already in memory:

    GET /api/version          the probe `Core::set_server` reads
    GET /api/stream/<id>      a FLAC body (Range supported, so a seek works)
    GET /api/cover/<id>       a 1×1 PNG — enough for `NzBitmapLoader` to decode,
                              which is what the notification's artwork test asks

Nothing else is served. The bytes are `feature/entity`'s `tone.flac` by default
(a real FLAC, so the engine's clock moves for a real reason), and the emulator
reaches this on `10.0.2.2:<port>`.

    adb shell am instrument -e nzServer http://10.0.2.2:4711 …

`-no-audio` still gives the guest a working AAudio clock (MQ1 measured it), so
"the clock advanced" is a real playback figure and never a host sound.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PNG_1X1 = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="
)

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
DEFAULT_FLAC = os.path.join(
    ROOT, "mobile", "android", "feature", "entity", "src", "androidTest", "assets", "tone.flac"
)


class Handler(BaseHTTPRequestHandler):
    flac: bytes = b""

    def log_message(self, *_args) -> None:  # quiet: a bench log is not a chat
        pass

    def do_GET(self) -> None:  # noqa: N802 (http.server's spelling)
        path = self.path.split("?", 1)[0]
        if path == "/api/version":
            body = json.dumps({"version": "0.1.0"}).encode()
            self._send(200, "application/json", body)
            return
        if path.startswith("/api/cover/"):
            self._send(200, "image/png", PNG_1X1)
            return
        if path.startswith("/api/stream/"):
            start, end = 0, len(self.flac) - 1
            code = 200
            header = self.headers.get("Range", "")
            if header.startswith("bytes="):
                first, _, last = header[len("bytes=") :].partition("-")
                if first.isdigit():
                    start = int(first)
                if last.isdigit():
                    end = min(int(last), end)
                code = 206
            body = self.flac[start : end + 1]
            self.send_response(code)
            self.send_header("Content-Type", "audio/flac")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Accept-Ranges", "bytes")
            if code == 206:
                self.send_header("Content-Range", f"bytes {start}-{end}/{len(self.flac)}")
            self.end_headers()
            self.wfile.write(body)
            return
        self._send(404, "text/plain", b"not here")

    def _send(self, code: int, kind: str, body: bytes) -> None:
        self.send_response(code)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=4711)
    parser.add_argument("--flac", default=DEFAULT_FLAC)
    args = parser.parse_args()
    if not os.path.isfile(args.flac):
        parser.error(f"no FLAC at {args.flac}")
    Handler.flac = open(args.flac, "rb").read()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"tone server on 127.0.0.1:{args.port} ({len(Handler.flac)} bytes)", flush=True)
    server.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
