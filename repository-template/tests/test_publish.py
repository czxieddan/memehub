import tomllib
import unittest

from support import BASE, PNG, REPO, FakeGitHub, entry, manifests
from memehub.protocol import Invalid
from memehub.publish import publish


class PublishTests(unittest.TestCase):
    def test_only_index_is_created_source_is_parent_and_rerun_noops(self):
        base = manifests({**entry(), **entry(user=9, data=PNG + b"other")})
        api = FakeGitHub(base=base)
        sha = publish(api, REPO)
        self.assertEqual(set(api.writes[0][2]), {"manifest/index.toml"})
        fields = tomllib.loads(api.commits[sha]["manifest/index.toml"].decode())
        self.assertEqual(set(fields), {"schema_version", "source_commit", "hashes"})
        self.assertEqual(fields["source_commit"], BASE)
        self.assertEqual(fields["hashes"], sorted(fields["hashes"]))
        self.assertEqual(len(fields["hashes"]), 2)
        self.assertEqual(publish(api, REPO), sha)
        self.assertEqual(len(api.writes), 1)

    def test_empty_repository_publishes_explicit_empty_hashes(self):
        api = FakeGitHub()
        sha = publish(api, REPO)
        fields = tomllib.loads(api.commits[sha]["manifest/index.toml"].decode())
        self.assertEqual(fields, {"schema_version": 1, "source_commit": BASE, "hashes": []})

    def test_missing_manifest_is_never_repaired_on_main(self):
        api = FakeGitHub(base=entry())
        with self.assertRaises(Invalid):
            publish(api, REPO)
        self.assertFalse(api.writes)
        self.assertEqual(api.refs[(REPO, "main")], BASE)

    def test_invalid_main_preserves_old_index(self):
        base = manifests(entry())
        base["manifest/index.toml"] = b'old index remains untouched'
        base.update(entry(data=PNG + b"changed without manifest"))
        api = FakeGitHub(base=base)
        with self.assertRaises(Invalid):
            publish(api, REPO)
        self.assertFalse(api.writes)
        self.assertEqual(api.commits[BASE]["manifest/index.toml"], b"old index remains untouched")

    def test_cas_race_reloads_current_main_and_retries_without_force(self):
        api = FakeGitHub(base=manifests(entry()))
        newer = "5" * 40
        api.add_commit(newer, manifests(entry(data=PNG + b"new main")))
        def race_once(client):
            client.refs[(REPO, "main")] = newer
            client.on_update = None
        api.on_update = race_once
        sha = publish(api, REPO)
        self.assertEqual(len(api.writes), 2)
        fields = tomllib.loads(api.commits[sha]["manifest/index.toml"].decode())
        self.assertEqual(fields["source_commit"], newer)
        self.assertEqual(api.writes[-1][1], newer)

    def test_repeated_races_are_bounded(self):
        api = FakeGitHub()
        def race(client):
            counter = len(client.writes)
            newer = str(counter + 3) * 40
            client.add_commit(newer, {"README.md": str(counter).encode()})
            client.refs[(REPO, "main")] = newer
        api.on_update = race
        with self.assertRaisesRegex(Invalid, "3 次"):
            publish(api, REPO)
        self.assertEqual(len(api.writes), 3)


if __name__ == "__main__":
    unittest.main()
