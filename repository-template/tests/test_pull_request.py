import hashlib
import unittest

from support import BASE, HEAD, PNG, REPO, FakeGitHub, entry, manifests
from memehub.cli import revalidate
from memehub.protocol import Duplicate, Invalid
from memehub.pull_request import Config, Validator, uploader
from memehub.repository import read_tree


class PullRequestTests(unittest.TestCase):
    def test_generates_source_branch_only_and_rerun_is_idempotent(self):
        api = FakeGitHub(head=entry())
        sha = Validator(api, REPO).run(1)
        self.assertNotEqual(sha, HEAD)
        self.assertEqual(api.refs[(REPO, "main")], BASE)
        self.assertEqual(api.refs[(REPO, "submission")], sha)
        self.assertNotIn("manifest/index.toml", api.writes[0][2])
        self.assertEqual(len(api.writes[0][2]), 1)
        self.assertEqual(api.statuses[-1][1:3], (sha, "success"))
        self.assertEqual(api.statuses[-1][4], "memehub/validate")
        self.assertEqual(Validator(api, REPO).run(1), sha)
        self.assertEqual(len(api.writes), 1)
        read_tree(api, REPO, sha).validated()

    def test_duplicate_comments_concrete_existing_path_and_closes_entire_pr(self):
        base = manifests(entry(user=9, name="Existing"))
        api = FakeGitHub(base, {**base, **entry()})
        with self.assertRaises(Duplicate):
            Validator(api, REPO).run(1)
        self.assertEqual(api.closed, [1])
        self.assertIn("/data/9/Existing/meme.png", api.comments[-1])
        self.assertIn("/data/7/Pig/meme.png", api.comments[-1])
        self.assertIn(hashlib.sha256(PNG).hexdigest(), api.comments[-1])
        self.assertFalse(api.writes)

    def test_duplicate_in_one_pr_same_author_different_name(self):
        api = FakeGitHub(head={**entry(), **entry(name="Other")})
        with self.assertRaises(Duplicate):
            Validator(api, REPO).run(1)
        self.assertEqual(api.closed, [1])

    def test_update_own_hash_is_not_duplicate_and_updates_unicode_metadata(self):
        base = manifests(entry())
        api = FakeGitHub(base, {**base, **entry(display="改名 🐷")})
        sha = Validator(api, REPO).run(1)
        items = read_tree(api, REPO, sha).validated()
        self.assertEqual(items["7/Pig"].digest, hashlib.sha256(PNG).hexdigest())
        self.assertEqual(items["7/Pig"].display_name, "改名 🐷")
        self.assertFalse(api.closed)

    def test_replacement_deletes_obsolete_manifest(self):
        base = manifests(entry())
        old = next(path for path in base if path.startswith("manifest/"))
        api = FakeGitHub(base, {**base, **entry(data=PNG + b"changed")})
        sha = Validator(api, REPO).run(1)
        self.assertIsNone(api.writes[-1][2][old])
        self.assertNotIn(old, api.commits[sha])
        read_tree(api, REPO, sha).validated()
        self.assertEqual(Validator(api, REPO).run(1), sha)

    def test_delete_entry_removes_manifest_and_remains_idempotent(self):
        base = manifests(entry())
        api = FakeGitHub(base, {p: data for p, data in base.items() if p.startswith("manifest/")})
        sha = Validator(api, REPO).run(1)
        self.assertEqual(api.commits[sha], {})
        self.assertEqual(Validator(api, REPO).run(1), sha)

    def test_stale_head_does_not_comment_or_close(self):
        base = manifests(entry(user=9))
        api = FakeGitHub(base, {**base, **entry()})
        def change_head(client):
            if client.pull_reads == 2:
                client.pr["head"]["sha"] = "3" * 40
        api.on_pull = change_head
        with self.assertRaises(Duplicate):
            Validator(api, REPO).run(1)
        self.assertFalse(api.closed)
        self.assertFalse(api.comments)

    def test_head_changes_after_comment_guard_prevents_close(self):
        base = manifests(entry(user=9))
        api = FakeGitHub(base, {**base, **entry()})
        api.on_comment = lambda client: client.pr["head"].update(sha="3" * 40)
        with self.assertRaises(Duplicate):
            Validator(api, REPO).run(1)
        self.assertEqual(len(api.comments), 1)
        self.assertFalse(api.closed)

    def test_author_mismatch_rejected_and_marker_not_trusted_for_humans(self):
        api = FakeGitHub(head=entry(user=121408646))
        api.pr["body"] = "<!-- memehub-uploader:121408646 -->\nCo-authored-by: someone"
        with self.assertRaisesRegex(Invalid, "投稿者 7"):
            Validator(api, REPO, Config(bot_login="website[bot]")).run(1)
        self.assertFalse(api.writes)
        self.assertFalse(api.closed)

    def test_configured_bot_marker_and_only_that_bot(self):
        api = FakeGitHub(head=entry(user=121408646))
        api.pr["user"] = {"id": 99, "type": "Bot", "login": "website[bot]"}
        api.pr["body"] = "<!-- memehub-uploader:121408646 -->"
        self.assertEqual(uploader(api.pr, Config(bot_login="other[bot]")), 99)
        sha = Validator(api, REPO, Config(bot_login="website[bot]")).run(1)
        self.assertEqual(read_tree(api, REPO, sha).validated()["121408646/Pig"].uploader_id, 121408646)
        api.pr["body"] *= 2
        with self.assertRaises(Invalid):
            uploader(api.pr, Config(bot_login="website[bot]"))

    def test_maintainer_import_requires_variable_and_verified_repository_permission(self):
        api = FakeGitHub(head=entry(user=9))
        api.pr["author_association"] = "MEMBER"
        api.user_permission = "maintain"
        with self.assertRaises(Invalid):
            Validator(api, REPO).run(1)
        Validator(api, REPO, Config(allow_maintainer_import=True)).run(1)
        outsider = FakeGitHub(head=entry(user=9))
        outsider.pr["author_association"] = "MEMBER"
        outsider.user_permission = "write"
        with self.assertRaises(Invalid):
            Validator(outsider, REPO, Config(allow_maintainer_import=True)).run(1)

    def test_fork_without_installation_has_concrete_error_no_main_fallback(self):
        api = FakeGitHub(head=entry())
        api.pr["head"]["repo"]["full_name"] = "outsider/images"
        with self.assertRaisesRegex(Invalid, "outsider/images.*fork 必须安装"):
            Validator(api, REPO).run(1)
        self.assertEqual(api.refs[(REPO, "main")], BASE)
        self.assertFalse(api.writes)

    def test_fork_installed_writer_only_updates_fork(self):
        api = FakeGitHub(head=entry())
        fork = "outsider/images"
        api.pr["head"]["repo"]["full_name"] = fork
        api.refs[(fork, "submission")] = HEAD
        sha = Validator(api, REPO, head_api=api).run(1)
        self.assertEqual(api.writes[0][0], fork)
        self.assertEqual(api.refs[(fork, "submission")], sha)
        self.assertEqual(api.refs[(REPO, "main")], BASE)
        self.assertEqual(Validator(api, REPO).run(1), sha)

    def test_http_fork_permission_error_is_actionable(self):
        api = FakeGitHub(head=entry())
        api.pr["head"]["repo"]["full_name"] = "outsider/images"
        api.write_error = 403
        with self.assertRaisesRegex(Invalid, "contents/statuses"):
            Validator(api, REPO, head_api=api).run(1)

    def test_reject_input_edits_to_manifests_index_and_workflow(self):
        cases = [manifests(entry()), {**entry(), "manifest/index.toml": b"hashes=[]"},
                 {**entry(), ".github/workflows/untrusted.yml": b"anything"}]
        for files in cases:
            with self.subTest(files=list(files)):
                api = FakeGitHub(head=files)
                with self.assertRaises(Invalid):
                    Validator(api, REPO).run(1)
                self.assertFalse(api.writes)

    def test_main_push_revalidates_old_pr_and_closes_new_duplicate(self):
        api = FakeGitHub(head=entry())
        Validator(api, REPO).run(1)
        newbase = "4" * 40
        api.add_commit(newbase, manifests(entry(user=9, name="MergedFirst")))
        api.refs[(REPO, "main")] = newbase
        with self.assertRaises(Invalid):
            revalidate(api, REPO, Config())
        self.assertEqual(api.closed, [1])
        self.assertIn("MergedFirst", api.comments[-1])

    def test_broken_base_is_not_a_reason_to_close_pr(self):
        broken = manifests({**entry(user=9), **entry(user=10)})
        api = FakeGitHub(broken, {**broken, **entry(data=PNG + b"new")})
        with self.assertRaisesRegex(Invalid, "默认分支内容无效"):
            Validator(api, REPO).run(1)
        self.assertFalse(api.closed)

    def test_fork_local_generation_receipt_cannot_authorize_manifest_input(self):
        api = FakeGitHub(head=manifests(entry()))
        fork = "outsider/images"
        api.pr["head"]["repo"]["full_name"] = fork
        api.receipts.add((fork, HEAD))
        with self.assertRaisesRegex(Invalid, "禁止投稿者编辑生成文件"):
            Validator(api, REPO).run(1)
        self.assertFalse(api.writes)

    def test_contributor_edit_after_generation_is_rejected(self):
        api = FakeGitHub(head=entry())
        generated = Validator(api, REPO).run(1)
        files = dict(api.commits[generated])
        path = next(path for path in files if path.startswith("manifest/"))
        files[path] += b"# contributor edit\n"
        changed = "7" * 40
        api.add_commit(changed, files)
        api.pr["head"]["sha"] = changed
        api.refs[(REPO, "submission")] = changed
        with self.assertRaisesRegex(Invalid, "禁止投稿者编辑生成文件"):
            Validator(api, REPO).run(1)
        self.assertEqual(len(api.writes), 1)


if __name__ == "__main__":
    unittest.main()
