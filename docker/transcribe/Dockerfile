# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# whisper.cpp v1.9.5, CPU only. GGML_NATIVE=OFF forbids -march=native; on x86-64
# the default build still enables SSE4.2/AVX/AVX2/FMA/F16C (Haswell, 2013+).
# BUILD_SHARED_LIBS=OFF: the runtime image carries no shared whisper libraries.
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS whisper

ARG WHISPER_VERSION=1.9.5
ARG WHISPER_SHA256=ff1a9053feb509ff9d7729703355541ae9690073a6b1c40eb692c962e0dc1720

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential ca-certificates cmake curl \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
RUN curl -fsSL -o whisper.tar.gz \
      "https://github.com/ggml-org/whisper.cpp/archive/refs/tags/v${WHISPER_VERSION}.tar.gz" \
 && echo "${WHISPER_SHA256}  whisper.tar.gz" | sha256sum -c - \
 && tar xzf whisper.tar.gz --strip-components=1 \
 && rm whisper.tar.gz

RUN cmake -S . -B build \
      -DCMAKE_BUILD_TYPE=Release \
      -DBUILD_SHARED_LIBS=OFF \
      -DGGML_NATIVE=OFF \
      -DGGML_CCACHE=OFF \
      -DWHISPER_BUILD_IS_DEV=OFF \
      -DWHISPER_BUILD_TESTS=OFF \
      -DWHISPER_BUILD_EXAMPLES=ON \
      -DWHISPER_BUILD_SERVER=OFF \
 && cmake --build build --config Release --target whisper-cli -j"$(nproc)" \
 && install -d /out \
 && install -m 0755 build/bin/whisper-cli /out/whisper-cli

# ---------------------------------------------------------------------------
# One static ffmpeg, the same multi-arch pin as the API image.
# ---------------------------------------------------------------------------
FROM mwader/static-ffmpeg:9.0.2@sha256:7d9bdaaf887f7e6ce6151f67325c344074b5ff1fb75316011c3376503e449a7b AS ffmpeg

# ---------------------------------------------------------------------------
# Runtime: Python + requests, whisper-cli, ffmpeg, no model, no shell daemon.
# ---------------------------------------------------------------------------
FROM python:3.12-slim-bookworm AS runtime

LABEL org.opencontainers.image.title="Neezer transcriber" \
      org.opencontainers.image.description="Neezer transcription sidecar: whisper.cpp on CPU, no database, no archive mount" \
      org.opencontainers.image.source="https://github.com/fnyaker/Neezer" \
      org.opencontainers.image.licenses="AGPL-3.0-only"

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_NO_CACHE_DIR=1 \
    HOME=/tmp \
    TMPDIR=/tmp

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates libgomp1 libstdc++6 \
 && rm -rf /var/lib/apt/lists/*

COPY --from=ffmpeg /ffmpeg /usr/local/bin/ffmpeg
COPY --from=whisper /out/whisper-cli /usr/local/bin/whisper-cli

WORKDIR /app
COPY requirements.txt ./
RUN pip install -r requirements.txt \
 && python -m pip uninstall -y -q pip

COPY worker.py ./
COPY --chmod=0755 entrypoint.sh /usr/local/bin/neezer-transcribe

RUN groupadd --gid 10001 transcribe \
 && useradd --uid 10001 --gid 10001 --no-create-home \
      --home-dir /tmp --shell /usr/sbin/nologin transcribe

USER 10001:10001
WORKDIR /app

HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --retries=3 \
  CMD ["python3", "/app/worker.py", "--check"]

ENTRYPOINT ["/usr/local/bin/neezer-transcribe"]
