# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Every `ServiceError` message is a sentence this server wrote.

`py/stack-trace-exposure` has no guard (see `docs/agents/server.md`): a response answers with a
constant, never with `str(exc)`. The four places of `neezer/webui/` that answer a caught
`ServiceError` with its own message are closed as false positives on that promise, and this test is
what keeps the promise true: a `raise ServiceError(...)` may only be handed a literal sentence, or an
f-string whose only hole is an integer status code. Anything else — `str(exc)`, a URL, a provider's
body — fails here, so the dismissal stops being honest the moment one is added.
"""

import ast
import os
import unittest

SERVICES = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "neezer", "services"
)
RAISED = ("ServiceError", "ServiceAuthError")


def _name_of(node):
    if isinstance(node, ast.Call):
        node = node.func
    if isinstance(node, ast.Attribute):
        return node.attr
    return getattr(node, "id", None)


def _raises(tree):
    """Every `raise ServiceError…` of the module, call or bare class."""
    for node in ast.walk(tree):
        if isinstance(node, ast.Raise) and node.exc is not None:
            if _name_of(node.exc) in RAISED:
                yield node


def _says_a_sentence(node) -> bool:
    """A literal, or an f-string whose only hole is an integer status code."""
    if isinstance(node, ast.Constant):
        return isinstance(node.value, str)
    if isinstance(node, ast.JoinedStr):
        for part in node.values:
            if not isinstance(part, ast.FormattedValue):
                continue
            hole = part.value
            # `f"Spotify a répondu {r.status_code}"` — an int, never an object.
            if not (isinstance(hole, ast.Attribute) and hole.attr == "status_code"):
                return False
            if part.format_spec is not None:
                return False
        return True
    return False


class ServiceErrorMessageTestCase(unittest.TestCase):
    def test_every_raise_says_a_literal_sentence(self):
        checked = 0
        for name in sorted(os.listdir(SERVICES)):
            if not name.endswith(".py"):
                continue
            path = os.path.join(SERVICES, name)
            with open(path, encoding="utf-8") as fh:
                tree = ast.parse(fh.read(), path)
            for node in _raises(tree):
                self.assertIsInstance(
                    node.exc,
                    ast.Call,
                    f"{name}:{node.lineno} raises the class, saying nothing",
                )
                self.assertTrue(
                    node.exc.args, f"{name}:{node.lineno} raises without saying why"
                )
                for arg in node.exc.args:
                    self.assertTrue(
                        _says_a_sentence(arg),
                        f"{name}:{node.lineno} hands a raise something other than a "
                        f"literal: {ast.dump(arg)[:140]}",
                    )
                checked += 1
        # A rename of the exception class would make this test vacuous.
        self.assertGreaterEqual(
            checked, 20, f"only {checked} raises found under neezer/services/"
        )


if __name__ == "__main__":
    unittest.main()
