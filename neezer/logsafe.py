# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Keep request-supplied text from forging log lines.

Log calls interpolate names, titles, ids and paths that a client chose; one
carrying a newline would start a line of its own in the log, looking exactly
like one the server wrote, and one carrying ESC would repaint (or clear) the
terminal that displays it. Rather than wrap every such call, the arguments of
every record this package logs have their control characters escaped once,
where the record is made. The format strings are the code's own and are left
alone, as are tracebacks."""

import logging

_PACKAGE = __package__ or "neezer"
_TABLE = str.maketrans(
    {
        # The two that start a line of their own keep the readable escape.
        "\r": "\\r",
        "\n": "\\n",
        # Everything else a terminal (or a log reader) ACTS on instead of
        # printing: C0, DEL and C1. ESC starts an ANSI sequence that erases the
        # screen or rewrites the line — the same spoofing as a forged entry,
        # without the newline — and NUL truncates in several readers.
        **{chr(cp): "\\x%02x" % cp for cp in range(0x20) if cp not in (10, 13)},
        "\x7f": "\\x7f",
        **{chr(cp): "\\x%02x" % cp for cp in range(0x80, 0xA0)},
        # The Unicode line separators a log viewer may break on too.
        "\u2028": "\\u2028",
        "\u2029": "\\u2029",
    }
)


def _clean(value):
    if isinstance(value, str):
        return value.translate(_TABLE)
    # An exception's message often quotes what it was handed.
    if isinstance(value, BaseException):
        return str(value).translate(_TABLE)
    return value


def install():
    """Wrap the record factory (idempotent)."""
    previous = logging.getLogRecordFactory()
    if getattr(previous, "_neezer_logsafe", False):
        return

    def factory(*args, **kwargs):
        record = previous(*args, **kwargs)
        if record.args and (
            record.name == _PACKAGE or record.name.startswith(_PACKAGE + ".")
        ):
            if isinstance(record.args, tuple):
                record.args = tuple(_clean(a) for a in record.args)
            elif isinstance(record.args, dict):
                record.args = {k: _clean(v) for k, v in record.args.items()}
        return record

    factory._neezer_logsafe = True
    logging.setLogRecordFactory(factory)
