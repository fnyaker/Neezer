# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Copyright (C) 2018-2022 Alban 'spl0k' Féron
#
# Distributed under terms of the GNU AGPLv3 license.

from . import api
from .exceptions import GenericError

methods = (
    "getVideos",
    "getAvatar",
    "getShares",
    "createShare",
    "updateShare",
    "deleteShare",
    "hls",
)


def unsupported():
    return GenericError("Not supported by Neezer")


for m in methods:
    api.add_url_rule(f"/{m}", "unsupported", unsupported, methods=["GET", "POST"])
    api.add_url_rule(f"/{m}.view", "unsupported", unsupported, methods=["GET", "POST"])
