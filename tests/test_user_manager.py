# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The one door that creates an account: folded keys, validation, duplicates."""

import os
import tempfile
import threading
import unittest

from neezer import db
from neezer.managers.user import (
    InvalidUserName,
    UserManager,
    UserNameTaken,
    fold_name,
    validate_name,
)


class UserNamePolicyTestCase(unittest.TestCase):
    def test_fold_drops_case_accents_and_edges(self):
        self.assertEqual(fold_name("Élodie"), "elodie")
        self.assertEqual(fold_name("Jean-Luc"), "jean-luc")
        self.assertEqual(fold_name("  ALICE "), "alice")
        self.assertEqual(fold_name("Álice"), "alice")

    def test_the_shape_is_three_to_thirty_two_characters(self):
        self.assertEqual(validate_name("abc"), "abc")
        self.assertEqual(validate_name("a" * 32), "a" * 32)
        for name in ("", "a", "ab", "a" * 33, "a b", "../evil", "-abc", "abc-", "abc."):
            with self.assertRaises(InvalidUserName, msg=name):
                validate_name(name)
        self.assertRaises(TypeError, validate_name, None)

    def test_reserved_names_are_refused_only_when_asked(self):
        self.assertEqual(validate_name("admin"), "admin")
        for name in ("admin", "administrator", "root", "api", "server", "null"):
            with self.assertRaises(InvalidUserName, msg=name):
                validate_name(name, reserved=True)


class UserManagerAccountTestCase(unittest.TestCase):
    def setUp(self):
        self.fd, self.path = tempfile.mkstemp(suffix=".db")
        db.init_database("sqlite:///" + self.path)

    def tearDown(self):
        db.release_database()
        os.close(self.fd)
        try:
            os.remove(self.path)
        except OSError:
            pass

    def test_add_folds_the_key_and_keeps_the_password_working(self):
        user = UserManager.add("Élodie", "Alic3pass")
        self.assertEqual(user.name, "Élodie")
        self.assertEqual(user.name_key, "elodie")
        self.assertEqual(user.onboarding_step, "done")
        self.assertFalse(user.disabled)
        self.assertIsNone(user.last_login)
        self.assertIsNotNone(UserManager.try_auth("Élodie", "Alic3pass"))
        self.assertEqual(db.User.get(db.User.name == "Élodie").id, user.id)

    def test_add_refuses_a_folded_duplicate(self):
        UserManager.add("Alice", "Alic3pass")
        for name in ("alice", "ALICE", "Álice"):
            with self.assertRaises(UserNameTaken, msg=name):
                UserManager.add(name, "An0therpass")
        self.assertEqual(db.User.select().count(), 1)

    def test_add_accepts_a_reserved_name_for_the_admin_and_cli(self):
        user = UserManager.add("admin", "Adm1npass", admin=True)
        self.assertEqual(user.name_key, "admin")
        self.assertTrue(user.admin)

    def test_add_refuses_an_invalid_name(self):
        for name in ("ab", "a" * 33, "a b", "../evil"):
            with self.assertRaises(InvalidUserName, msg=name):
                UserManager.add(name, "Alic3pass")
        self.assertEqual(db.User.select().count(), 0)

    def test_the_creatable_fields_whitelist_is_unchanged(self):
        with self.assertRaises(ValueError):
            UserManager.add("alice", "Alic3pass", disabled=True)
        self.assertEqual(db.User.select().count(), 0)

    def test_add_cost_is_two_statements_and_a_duplicate_one(self):
        from .test_deezer import count_statements

        UserManager.add("warm", "Warm1pass")
        with count_statements() as counts:
            UserManager.add("alice", "Alic3pass")
        self.assertEqual(sum(counts.values()), 2)
        with count_statements() as counts:
            with self.assertRaises(UserNameTaken):
                UserManager.add("alice", "Alic3pass")
        self.assertEqual(sum(counts.values()), 1)
        alice = db.User.get(db.User.name == "alice")
        with count_statements() as counts:
            UserManager.rename(alice, "alice2")
        self.assertEqual(sum(counts.values()), 2)

    def test_concurrent_add_with_the_same_name_creates_one_row(self):
        results = []
        barrier = threading.Barrier(8)

        def create():
            barrier.wait()
            try:
                UserManager.add("dup", "An0therpass")
                results.append("ok")
            except UserNameTaken:
                results.append("taken")

        threads = [threading.Thread(target=create) for _ in range(8)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()

        self.assertEqual(results.count("ok"), 1)
        self.assertEqual(results.count("taken"), 7)
        self.assertEqual(db.User.select().count(), 1)
        self.assertEqual(db.User.get().name_key, "dup")

    def test_rename_moves_the_key_and_refuses_duplicates(self):
        alice = UserManager.add("Alice", "Alic3pass")
        UserManager.add("bob", "B0bpass")

        UserManager.rename(alice, "Carol")
        renamed = db.User.get(db.User.id == alice.id)
        self.assertEqual((renamed.name, renamed.name_key), ("Carol", "carol"))
        self.assertIsNotNone(UserManager.try_auth("Carol", "Alic3pass"))

        UserManager.rename("CAROL", "dave")
        moved = db.User.get(db.User.id == alice.id)
        self.assertEqual((moved.name, moved.name_key), ("dave", "dave"))

        with self.assertRaises(UserNameTaken):
            UserManager.rename(alice, "bob")
        with self.assertRaises(InvalidUserName):
            UserManager.rename(alice, "b b")

    def test_rename_by_name_can_normalise_an_old_display_name(self):
        user = UserManager.add("Alice", "Alic3pass")
        UserManager.rename("alice", "alice")
        again = db.User.get(db.User.id == user.id)
        self.assertEqual((again.name, again.name_key), ("alice", "alice"))

    def test_a_legacy_name_without_a_key_reserves_its_fold(self):
        db.User.create(name="Legacy", password="x", salt="xxxxxx")
        with self.assertRaises(UserNameTaken):
            UserManager.add("LEGACY", "Alic3pass")
        with self.assertRaises(InvalidUserName):
            UserManager.add("Legacy Name", "Alic3pass")


if __name__ == "__main__":
    unittest.main()
