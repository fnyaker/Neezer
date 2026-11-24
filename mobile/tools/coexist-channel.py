#!/usr/bin/env python3
"""A local Neezer server whose TWO Android channels point at test APKs.

`mobile/tools/coexist.sh` starts this, downloads each channel through its real
route, and installs the result on the emulator:

    /api/android/apk              -> the WebView app's neezer.apk (unchanged)
    /api/android/native-apk?abi=  -> the native app's neezer-native.apk

The two `[webapp] android_*` local-file settings are what make it a test server:
no GitHub release is touched, and the bytes served are the exact APKs handed to
the script. A `bench` account is created so the login-protected routes answer,
exactly as a person's server would.

    PYTHON=/path/to/venv/bin/python mobile/tools/coexist-channel.py \\
        --old-apk android/app/build/outputs/apk/release/app-release.apk \\
        --native-apk mobile/android/app/build/outputs/apk/release/app-universal-release.apk \\
        --port 5799

It prints `listening on 127.0.0.1:<port>` when Flask is up and runs until
SIGTERM. State (SQLite, cache) lives beside it, under `--state-dir`, never in
`/tmp` (a tmpfs on this machine).
"""

from __future__ import annotations

import argparse
import os
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, ROOT)

from neezer.config import DefaultConfig  # noqa: E402
from neezer.managers.user import UserManager  # noqa: E402
from neezer.web import create_application  # noqa: E402

BENCH_USER = "bench"
BENCH_PASSWORD = "Bench1"
# Declared versions: they only have to be tags this server can name. The APK
# bytes are the ones handed in, and `X-NS-Version` echoes these.
OLD_VERSION = "9.0.0"
NATIVE_VERSION = "9.1.0"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--old-apk", required=True)
    parser.add_argument("--native-apk", required=True)
    parser.add_argument("--port", type=int, default=5799)
    parser.add_argument("--state-dir", required=True)
    args = parser.parse_args()

    for label, path in (("old", args.old_apk), ("native", args.native_apk)):
        if not os.path.isfile(path):
            parser.error(f"--{label}-apk is not a file: {path}")

    work = os.path.abspath(args.state_dir)
    os.makedirs(work, exist_ok=True)
    db = os.path.join(work, "db.sqlite")
    cache = os.path.join(work, "cache")
    archive = os.path.join(work, "archive")
    os.makedirs(cache, exist_ok=True)
    os.makedirs(archive, exist_ok=True)

    class Config(DefaultConfig):
        def __init__(self):
            super().__init__()
            self.BASE = dict(self.BASE, database_uri="sqlite:///" + db)
            self.WEBAPP = dict(
                self.WEBAPP,
                cache_dir=cache,
                mount_webui=True,
                mount_api=True,
                android_version=OLD_VERSION,
                android_apk=os.path.abspath(args.old_apk),
                android_url=None,
                android_native_version=NATIVE_VERSION,
                android_native_apk=os.path.abspath(args.native_apk),
            )
            self.DEEZER = dict(
                getattr(self, "DEEZER", {}), archive_dir=archive, enabled=False
            )

    app = create_application(Config())
    with app.app_context():
        UserManager.add(BENCH_USER, BENCH_PASSWORD, admin=True)

    # `threaded`: the APK fetch runs in its own worker while the route answers
    # `202`; a single-threaded dev server would also hold the download.
    print(f"listening on 127.0.0.1:{args.port}", flush=True)
    app.run(host="127.0.0.1", port=args.port, threaded=True, use_reloader=False)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
