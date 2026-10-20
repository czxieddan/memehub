import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import urllib.error

from support import BASE, REPO, FakeGitHub, entry
from memehub.github import (API_ROOT, ApiError, Budget, GitHub, MAX_RESPONSE,
                            NoRedirect, repository_path, trusted_receipt)
from memehub.protocol import Invalid, MAX_IMAGE, MAX_TREE
from memehub.repository import check_tree_row, overlay, read_tree


class TreeTests(unittest.TestCase):
    def test_truncated_tree_is_rejected(self):
        api = FakeGitHub()
        api.tree_override = {"truncated": True, "tree": []}
        with self.assertRaisesRegex(Invalid, "截断"):
            read_tree(api, REPO, BASE)

    def test_oversized_tree_and_duplicate_paths_are_rejected(self):
        api = FakeGitHub(base=entry())
        row = api.tree(REPO, BASE)["tree"][0]
        for rows in ([row] * (MAX_TREE + 1), [row, row]):
            api.tree_override = {"truncated": False, "tree": rows}
            with self.assertRaises(Invalid):
                read_tree(api, REPO, BASE)

    def test_modes_submodules_and_paths_fail_closed(self):
        row = {"path": "data/7/Pig/meme.png", "sha": "a" * 40, "mode": "100644", "type": "blob"}
        for mode, kind in (("120000", "blob"), ("160000", "commit"), ("100755", "blob")):
            with self.subTest(mode=mode), self.assertRaises(Invalid):
                check_tree_row({**row, "mode": mode, "type": kind})
        for path in ("/data/7/Pig/a.png", "data/../bad", "data//bad", "data\\bad", "x\nfile"):
            with self.subTest(path=path), self.assertRaises(Invalid):
                check_tree_row({**row, "path": path})
        self.assertFalse(check_tree_row({"path": "data", "mode": "040000", "type": "tree"}))

    def test_oversize_image_rejected_before_blob_download(self):
        api = FakeGitHub(base=entry())
        response = api.tree(REPO, BASE)
        response["tree"][0]["size"] = MAX_IMAGE + 1
        api.tree_override = response
        with self.assertRaisesRegex(Invalid, "大小"):
            read_tree(api, REPO, BASE)

    def test_metadata_and_image_conflicts_require_base_refresh(self):
        api = FakeGitHub(base=entry(display="base"), head=entry(display="PR"))
        ancestor = "6" * 40
        api.add_commit(ancestor, entry(display="ancestor"))
        with self.assertRaisesRegex(Invalid, "冲突"):
            overlay(read_tree(api, REPO, BASE), read_tree(api, REPO, ancestor),
                    read_tree(api, REPO, "2" * 40))


class Response:
    def __init__(self, data):
        self.data = data
        self.requested_size = None

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False

    def read(self, size):
        self.requested_size = size
        return self.data[:size]


class TransportTests(unittest.TestCase):
    def test_fixed_https_origin_timeout_and_no_redirect(self):
        client = GitHub("secret-not-logged")
        response = Response(b'{"default_branch":"main"}')
        with patch.object(client._opener, "open", return_value=response) as opened:
            client.repo(REPO)
        request = opened.call_args.args[0]
        self.assertTrue(request.full_url.startswith(API_ROOT + "/repos/"))
        self.assertEqual(opened.call_args.kwargs["timeout"], 20)
        with self.assertRaises(ApiError):
            NoRedirect().redirect_request(None, None, 302, "", {}, "https://attacker.invalid/")
        with self.assertRaises(Invalid):
            client.request("GET", "https://attacker.invalid/secret")
        for repo in ("../bad", "a/b/c", "https://attacker.invalid/x", "a/.."):
            with self.subTest(repo=repo), self.assertRaises(Invalid):
                repository_path(repo)

    def test_http_errors_do_not_disclose_tokens_or_response_body(self):
        client = GitHub("secret-token")
        with patch.object(client._opener, "open", side_effect=urllib.error.HTTPError(
                "https://api.github.com", 403, "secret-token", {}, None)):
            with self.assertRaises(ApiError) as caught:
                client.repo(REPO)
        self.assertNotIn("secret-token", str(caught.exception))
        self.assertEqual(caught.exception.status, 403)

    def test_http_response_and_total_budgets_are_bounded(self):
        client = GitHub("fake")
        response = Response(b"x" * (MAX_RESPONSE + 1))
        with patch.object(client._opener, "open", return_value=response):
            with self.assertRaises(Invalid):
                client.repo(REPO)
        self.assertEqual(response.requested_size, MAX_RESPONSE + 1)
        budget = Budget()
        budget.requests = 2048
        with self.assertRaises(Invalid):
            budget.check()
        budget = Budget()
        budget.started -= 541
        with self.assertRaises(Invalid):
            budget.check()

    def test_generation_receipt_requires_bot_identity_and_valid_description(self):
        receipt = {"context": "memehub/generated", "state": "success",
                   "description": "Generated from " + "a" * 40,
                   "creator": {"type": "Bot", "login": "publisher[bot]"}}
        self.assertTrue(trusted_receipt(receipt, "publisher[bot]"))
        self.assertFalse(trusted_receipt(receipt, "other[bot]"))
        forged = copy.deepcopy(receipt)
        forged["creator"]["type"] = "User"
        self.assertFalse(trusted_receipt(forged, "publisher[bot]"))
        self.assertFalse(trusted_receipt({**receipt, "description": "spoofed"}, "publisher[bot]"))

    def test_fork_file_history_uses_target_repository_receipts(self):
        client = GitHub("fake")
        receipt = {"context": "memehub/generated", "state": "success",
                   "description": "Generated from " + "a" * 40,
                   "creator": {"type": "Bot", "login": "publisher[bot]"}}
        combined = {"statuses": [{"context": "memehub/validate", "state": "success"}, receipt]}
        with patch.object(client, "request", side_effect=[[{"sha": "b" * 40}], combined]) as request:
            accepted = client.generated_file("fork/images", "c" * 40, "manifest/x/manifest.toml",
                                             "publisher[bot]", REPO)
        self.assertTrue(accepted)
        self.assertTrue(request.call_args_list[0].args[1].startswith("/repos/fork/images/commits?"))
        self.assertTrue(request.call_args_list[1].args[1].startswith("/repos/" + REPO + "/commits/"))
        self.assertTrue(request.call_args_list[1].args[1].endswith("/status?per_page=100"))

    def test_event_file_is_temporary_data_and_secrets_are_not_required(self):
        from memehub.cli import event_data
        with tempfile.TemporaryDirectory() as temporary:
            event = Path(temporary) / "event.json"
            event.write_text(json.dumps({"number": 1}), encoding="utf-8")
            with patch.dict("os.environ", {"GITHUB_EVENT_PATH": str(event)}):
                self.assertEqual(event_data(), {"number": 1})


if __name__ == "__main__":
    unittest.main()
