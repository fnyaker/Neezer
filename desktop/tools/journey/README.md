# D17 — the journey bench

The tools that measure the desktop app's definition of done, so a number in
`docs/agents/desktop.md` or a PR can be re-taken by anyone. Nothing here is
part of the app.

| File | What it does |
|---|---|
| `throttle.py` | A degraded link with **no root**: `tc netem` replacement (latency, bandwidth, loss) plus the per-request/byte table the acceptance wants. |
| `drive.py` | Speaks X11 (XTEST) to a real window: keys, clicks, drags, resize, hide, screenshots. Refuses any display below `:130`. |
| `bench.py` | The journey: `perf_api.py --serve` on the big synthetic library, our own Xvfb, the release app, every screen opened with `--route`, requests/bytes/RSS/frames/pictures per screen. |
| `soak.py` | One hour of playback, 40 bulk downloads and a real whisper transcription at the same time; fails on a single underrun. |
| `frame_stats.py` | p50/p95/max frame interval and build time from a `--frame-log` file. |
| `frame_bench.sh` | The same, on the person's real display — the only place a frame rate is a frame rate. |

## The degraded link, without root

`tc qdisc add dev … root netem delay 150ms rate 1mbit loss 2%` needs `CAP_NET_ADMIN`
and a network interface. `throttle.py` is an HTTP/1.1 proxy in user space that
sits in front of ONE upstream server and shapes every byte the app receives:

```sh
python desktop/tools/journey/throttle.py \
    --listen 127.0.0.1:18080 --upstream 127.0.0.1:5001 \
    --latency-ms 150 --rate-bps 1000000 --loss 0.02 \
    --stats 127.0.0.1:18081
```

* `--latency-ms 150` is one way, so an exchange answers in **300 ms RTT**.
* `--rate-bps 1000000` is **1 Mb/s = 125 kB/s** downstream, paced on a FIFO
  clock (a chunk occupies `bytes·8/rate` seconds; the next one waits).
* `--loss 0.02` delays **2 % of chunks by one retransmission timeout**
  (`--rto-ms`, 200 ms). A proxy cannot drop a TCP segment — the kernel would
  never retransmit it and the stream would arrive corrupted — so what is
  modelled is what a lost segment COSTS an HTTP exchange (an RTO stall), with
  the data intact. That is documented here and in the module, not hidden.
* The stats listener is separate and on `127.0.0.1`, so the measured traffic
  never crosses the shaping code: `/stats` answers
  `{requests, bytes, paths: {path: {requests, bytes, status, ms}}, exchanges: […]}`
  and `/reset` starts a new screen's window.

The app is pointed at `http://127.0.0.1:18080`; the cookie it already has is
written under that origin, so a session is restored through the shaped link.

## Running it

```sh
# the project's venv has the server; the bench needs python-xlib and ImageMagick
python3 -m venv /tmp/nz-bench && /tmp/nz-bench/bin/pip install python-xlib
NZ_XLIB=/tmp/nz-bench/bin/python \
NZ_PYTHON=~/…/Neezer/.venv/bin/python \
python desktop/tools/journey/bench.py \
    --sizes 2560x1600,1280x800 --shots /tmp/nz-shots --out /tmp/nz-journey.json
```

(`bench.py` imports `python-xlib` from whatever interpreter runs it; a copy of
`python-xlib` in the project venv is fine too.)

Add `--degraded` for the 300 ms / 1 Mb/s / 2 % link, and `--screens
home,library:favorites` to re-take only a few screens.

## What is measured, and what is not

* **Requests and bytes** are the proxy's, for the window between the session
  landing (`bench logged_in`) and 500 ms without a new request. Startup
  requests (`/api/me`, the reachability probe) land before that window, so a
  screen's row is the screen's own work. A sleeve cache warmed by an earlier
  screen changes a later row — that is the point: the table says so.
* **Frame time under Xvfb is a floor.** The app renders through `lavapipe`
  (Mesa's software Vulkan) there, and the present rate plateaus at 24–30 fps
  whatever it draws; the harness reports the app's own frame-build time, which
  is comparable across changes, and never calls the Xvfb rate an fps target.
  The real number comes from `frame_bench.sh` on the panel.
* **RSS is not the app's data.** The screens' table reports it because D17
  asks for it, next to the window size; the rule about bytes lives in
  `docs/agents/desktop.md` (*Memory*) and in `app/src/testmem.rs`.
* **The soak plays MUTED** (20 × `Ctrl+↓` before the first click) so the
  person's speakers stay silent while the exact same callback path runs.
