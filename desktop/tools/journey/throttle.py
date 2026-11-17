#!/usr/bin/env python3
"""A degraded link in user space: latency, bandwidth and loss, no root.

`tc netem` is the usual way to reproduce a bad mobile link, and it needs root
and a network interface. This proxy does the same job between a client and one
upstream HTTP server, so a measurement can be run by anybody:

    python desktop/tools/journey/throttle.py \
        --listen 18080 --upstream 127.0.0.1:5001 \
        --latency-ms 150 --rate-bps 1000000 --loss 0.02 --stats-port 18081

The app then talks to `http://127.0.0.1:18080` and every byte crosses the
proxy. `GET http://127.0.0.1:18081/stats` answers the per-request table the
D17 acceptance asks for (path, status, response bytes, time to last byte), and
`/reset` starts a new screen's window. Both live on a LISTENER OF THEIR OWN, on
127.0.0.1, so the measured traffic never goes through the shaping code.

# What each knob models, and why it is done this way

* **Latency** (`--latency-ms`, one way) is applied to every response chunk's
  send time on a FIFO clock: a chunk due at `now + latency`, and the next one no
  earlier than the previous one's end plus its own transmission time. That is a
  serial link with a propagation delay, which is what 300 ms RTT is.
* **Bandwidth** (`--rate-bps`) is the same clock: a chunk of *n* bytes occupies
  `n * 8 / rate` seconds. 1 Mb/s is 125 kB/s, so a 1 MB cover cache warms in
  8 s, not in one burst.
* **Loss** (`--loss`) is the honest approximation available above TCP: a proxy
  cannot drop a TCP *segment* (the kernel would never retransmit it, and the
  byte stream would arrive corrupted). What a 2 % loss costs an HTTP exchange is
  a retransmission timeout on the affected segments, so 2 % of the chunks are
  delayed by one retransmission timeout (`--rto-ms`, 200 ms by default) — the
  same delay a lost segment pays, with the data intact. The client sees stalls,
  not corruption.
* **No reordering, no duplication**: TCP does not deliver either upward, so
  modelling them upstream of TCP would be wrong.

Requests and responses are framed as HTTP/1.1 (Content-Length, chunked, or
read-until-close for a stream), so the bytes counted for a path are the bytes
of ITS response, gzip or not, and a keep-alive connection is one request per
exchange. The proxy is for measurement only: it does not cache, rewrite, or
authenticate anything.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import random
import time
from collections import deque

# How much of one response is read at a time on the wire. 16 KiB is a page of a
# FILE read; the point is that the shaping clock sees real chunks.
CHUNK = 16 * 1024

# A request line longer than this is not a request line: refuse it rather than
# grow a buffer for a hostile client.
MAX_LINE = 8 * 1024

# The headers of one request or response, bounded the same way.
MAX_HEADERS = 64 * 1024


class Link:
    """One direction of the shaped link: latency, rate, loss on a FIFO clock."""

    def __init__(self, latency_s: float, rate_bps: int, loss: float, rto_s: float,
                 rng: random.Random) -> None:
        self.latency_s = latency_s
        self.rate_bps = rate_bps
        self.loss = loss
        self.rto_s = rto_s
        self.rng = rng
        self.free_at = 0.0

    def plan(self, size: int) -> float:
        """When a chunk of `size` bytes will be handed to the other side."""
        now = time.monotonic()
        start = max(now + self.latency_s, self.free_at)
        if self.loss and self.rng.random() < self.loss:
            # A lost segment: TCP waits one RTO before sending it again.
            start += self.rto_s
        transfer = (size * 8.0 / self.rate_bps) if self.rate_bps > 0 else 0.0
        self.free_at = start + transfer
        return start


class Exchange:
    """One request/response pair, as the stats table reports it."""

    __slots__ = ("method", "path", "status", "started", "bytes", "first_byte", "last_byte")

    def __init__(self, method: str, path: str) -> None:
        self.method = method
        self.path = path
        self.status = 0
        self.started = time.monotonic()
        self.bytes = 0
        self.first_byte = 0.0
        self.last_byte = 0.0

    def as_dict(self) -> dict:
        return {
            "method": self.method,
            "path": self.path,
            "status": self.status,
            "bytes": self.bytes,
            "ms": round((self.last_byte - self.started) * 1000.0, 1),
            "ttfb_ms": round((self.first_byte - self.started) * 1000.0, 1)
            if self.first_byte
            else None,
        }


class Stats:
    def __init__(self) -> None:
        self.exchanges: list[Exchange] = []
        self.lock = asyncio.Lock()

    async def add(self, exchange: Exchange) -> None:
        async with self.lock:
            self.exchanges.append(exchange)

    async def snapshot(self) -> dict:
        async with self.lock:
            rows = [exchange.as_dict() for exchange in self.exchanges]
        by_path: dict[str, dict] = {}
        for row in rows:
            entry = by_path.setdefault(
                row["path"], {"requests": 0, "bytes": 0, "status": 0, "ms": 0.0}
            )
            entry["requests"] += 1
            entry["bytes"] += row["bytes"]
            entry["status"] = row["status"]
            entry["ms"] = max(entry["ms"], row["ms"])
        return {
            "requests": len(rows),
            "bytes": sum(row["bytes"] for row in rows),
            "paths": by_path,
            "exchanges": rows,
        }

    async def reset(self) -> None:
        async with self.lock:
            self.exchanges.clear()


async def read_line(reader: asyncio.StreamReader) -> bytes | None:
    try:
        return await reader.readuntil(b"\r\n")
    except (asyncio.IncompleteReadError, asyncio.LimitOverrunError):
        return None


async def read_headers(reader: asyncio.StreamReader) -> tuple[bytes, dict[str, str]] | None:
    """The raw header block and its parsed fields (lower-case names)."""
    block = b""
    headers: dict[str, str] = {}
    while True:
        line = await read_line(reader)
        if line is None:
            return None
        block += line
        if line in (b"\r\n", b"\n"):
            return block, headers
        if len(block) > MAX_HEADERS:
            return None
        text = line.decode("latin-1").rstrip("\r\n")
        if ":" in text:
            name, _, value = text.partition(":")
            headers[name.strip().lower()] = value.strip()


class Shaper:
    """The proxy's per-connection state machine."""

    def __init__(self, args, stats: Stats, rng: random.Random) -> None:
        self.args = args
        self.stats = stats
        self.up = Link(args.latency_ms / 1000.0, args.rate_bps, args.loss,
                       args.rto_ms / 1000.0, rng)
        self.down = Link(args.latency_ms / 1000.0, args.rate_bps, args.loss,
                         args.rto_ms / 1000.0, random.Random(rng.random()))

    async def handle(self, client: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        try:
            upstream_reader, upstream_writer = await asyncio.open_connection(
                self.args.upstream_host, self.args.upstream_port
            )
        except OSError:
            writer.close()
            return
        try:
            await self.pump(client, writer, upstream_reader, upstream_writer)
        except (ConnectionResetError, BrokenPipeError, asyncio.IncompleteReadError):
            pass
        finally:
            for stream in (writer, upstream_writer):
                try:
                    stream.close()
                except Exception:
                    pass

    async def pump(self, client, writer, upstream_reader, upstream_writer) -> None:
        while True:
            # --- one request: its start line, its headers, its body -----------
            request_line = await read_line(client)
            if request_line is None:
                return
            request = request_line.decode("latin-1").rstrip("\r\n")
            parts = request.split(" ")
            if len(parts) < 2 or len(request_line) > MAX_LINE:
                return
            method, path = parts[0], parts[1]
            exchange = Exchange(method, path)
            parsed = await read_headers(client)
            if parsed is None:
                return
            headers_block, headers = parsed
            body_length = int(headers.get("content-length", "0") or 0)
            if self.args.deny and path.startswith(self.args.deny):
                # An older server never had this route: Flask's own answer, with
                # no upstream call (so the measured server sees nothing).
                body = b"<!doctype html><html><body><h1>Not Found</h1></body></html>"
                writer.write(
                    b"HTTP/1.1 404 NOT FOUND\r\nContent-Type: text/html\r\n"
                    b"Content-Length: " + str(len(body)).encode() + b"\r\n"
                    b"Connection: keep-alive\r\n\r\n" + body
                )
                await writer.drain()
                exchange.status = 404
                exchange.bytes = len(body)
                exchange.first_byte = exchange.last_byte = time.monotonic()
                await self.stats.add(exchange)
                # Drain the request body, if any, so the next request parses.
                while body_length:
                    chunk = await client.read(min(CHUNK, body_length))
                    if not chunk:
                        return
                    body_length -= len(chunk)
                continue
            # Forward the request: headers at once (the upstream needs them to
            # answer), the body as it arrives.
            upstream_writer.write(request_line + headers_block)
            if body_length:
                while body_length:
                    chunk = await client.read(min(CHUNK, body_length))
                    if not chunk:
                        return
                    upstream_writer.write(chunk)
                    body_length -= len(chunk)
            await upstream_writer.drain()

            # --- its response ------------------------------------------------
            status_line = await read_line(upstream_reader)
            if status_line is None:
                return
            response_parsed = await read_headers(upstream_reader)
            if response_parsed is None:
                return
            response_headers_block, response_headers = response_parsed
            try:
                exchange.status = int(status_line.split(b" ")[1])
            except (IndexError, ValueError):
                exchange.status = 0

            writer.write(status_line + response_headers_block)
            exchange.bytes = len(status_line) + len(response_headers_block)
            exchange.first_byte = exchange.last_byte = time.monotonic()
            await writer.drain()

            connection = response_headers.get("connection", "").lower()
            close_after = connection == "close" or self.args.always_close
            transfer = response_headers.get("transfer-encoding", "").lower()

            if transfer == "chunked":
                await self.forward_chunked(upstream_reader, writer, exchange)
            elif "content-length" in response_headers:
                remaining = int(response_headers["content-length"])
                await self.forward_fixed(upstream_reader, writer, exchange, remaining)
            else:
                # No length: the body ends when the server closes the socket
                # (a stream generated as it is sent, `/api/stream`).
                await self.forward_until_close(upstream_reader, writer, exchange)
                close_after = True

            exchange.last_byte = time.monotonic()
            await self.stats.add(exchange)
            if close_after:
                return

    async def shaped_write(self, writer, data: bytes) -> None:
        due = self.down.plan(len(data))
        delay = due - time.monotonic()
        if delay > 0:
            await asyncio.sleep(delay)
        writer.write(data)
        await writer.drain()

    async def forward_fixed(self, reader, writer, exchange: Exchange, remaining: int) -> None:
        while remaining > 0:
            chunk = await reader.read(min(CHUNK, remaining))
            if not chunk:
                return
            remaining -= len(chunk)
            exchange.bytes += len(chunk)
            await self.shaped_write(writer, chunk)

    async def forward_until_close(self, reader, writer, exchange: Exchange) -> None:
        while True:
            chunk = await reader.read(CHUNK)
            if not chunk:
                return
            exchange.bytes += len(chunk)
            await self.shaped_write(writer, chunk)

    async def forward_chunked(self, reader, writer, exchange: Exchange) -> None:
        # The framing travels byte for byte; what is shaped and counted is the
        # whole thing, because the client reads the framing too.
        while True:
            size_line = await read_line(reader)
            if size_line is None:
                return
            exchange.bytes += len(size_line)
            await self.shaped_write(writer, size_line)
            try:
                size = int(size_line.split(b";")[0].strip(), 16)
            except ValueError:
                return
            remaining = size
            while remaining:
                chunk = await reader.read(min(CHUNK, remaining))
                if not chunk:
                    return
                remaining -= len(chunk)
                exchange.bytes += len(chunk)
                await self.shaped_write(writer, chunk)
            crlf = await read_line(reader)
            if crlf is None:
                return
            exchange.bytes += len(crlf)
            await self.shaped_write(writer, crlf)
            if size == 0:
                # The trailer block, if any, ends with an empty line.
                while True:
                    line = await read_line(reader)
                    if line is None:
                        return
                    exchange.bytes += len(line)
                    await self.shaped_write(writer, line)
                    if line in (b"\r\n", b"\n"):
                        return


def stats_handler(stats: Stats):
    async def handle(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        try:
            line = await asyncio.wait_for(read_line(reader), timeout=5)
        except asyncio.TimeoutError:
            writer.close()
            return
        if line is None:
            writer.close()
            return
        request = line.decode("latin-1")
        path = request.split(" ")[1] if len(request.split(" ")) > 1 else "/"
        if path.startswith("/reset"):
            await stats.reset()
            body = json.dumps({"reset": True}).encode()
        else:
            body = json.dumps(await stats.snapshot()).encode()
        # Drain the client's headers so the connection closes cleanly.
        while True:
            header = await read_line(reader)
            if header in (None, b"\r\n", b"\n"):
                break
        writer.write(
            b"HTTP/1.1 200 OK\r\n"
            b"Content-Type: application/json\r\n"
            b"Cache-Control: no-store\r\n"
            + b"Content-Length: " + str(len(body)).encode() + b"\r\n"
            b"Connection: close\r\n\r\n" + body
        )
        await writer.drain()
        writer.close()

    return handle


async def main_async(args) -> None:
    stats = Stats()
    rng = random.Random(args.seed)
    shaper = Shaper(args, stats, rng)

    server = await asyncio.start_server(shaper.handle, args.listen_host, args.listen_port)
    stats_server = await asyncio.start_server(
        stats_handler(stats), args.stats_host, args.stats_port
    )
    ports = [server.sockets[0].getsockname()[1], stats_server.sockets[0].getsockname()[1]]
    print("READY", ports[0], ports[1], flush=True)
    async with server, stats_server:
        await asyncio.gather(server.serve_forever(), stats_server.serve_forever())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--listen", default="127.0.0.1:18080",
                        help="where the app connects (host:port)")
    parser.add_argument("--upstream", required=True, help="the real server (host:port)")
    parser.add_argument("--stats", default="127.0.0.1:18081", help="stats listener (host:port)")
    parser.add_argument("--latency-ms", type=float, default=150.0,
                        help="one-way delay; 150 gives the 300 ms RTT of the acceptance")
    parser.add_argument("--rate-bps", type=int, default=1_000_000,
                        help="downstream bits per second (1 Mb/s = 125 kB/s)")
    parser.add_argument("--loss", type=float, default=0.0,
                        help="fraction of chunks that pay one retransmission timeout")
    parser.add_argument("--rto-ms", type=float, default=200.0,
                        help="what a lost segment costs (default 200 ms)")
    parser.add_argument("--seed", type=int, default=17, help="loss draws are reproducible")
    parser.add_argument("--always-close", action="store_true",
                        help="close after every exchange (a hostile keep-alive check)")
    parser.add_argument("--deny", default="",
                        help="answer 404 for paths starting with this prefix, without "
                             "forwarding: how an OLDER server, which never had the route, "
                             "looks to the client (D17's upgrade check)")
    args = parser.parse_args()

    listen_host, _, listen_port = args.listen.partition(":")
    stats_host, _, stats_port = args.stats.partition(":")
    upstream_host, _, upstream_port = args.upstream.partition(":")
    args.listen_host = listen_host or "127.0.0.1"
    args.listen_port = int(listen_port or 18080)
    args.stats_host = stats_host or "127.0.0.1"
    args.stats_port = int(stats_port or 18081)
    args.upstream_host = upstream_host or "127.0.0.1"
    args.upstream_port = int(upstream_port)
    try:
        asyncio.run(main_async(args))
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
