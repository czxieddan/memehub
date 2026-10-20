"""Small in-memory GitHub model: no sockets, credentials or working-tree writes."""

import copy
import hashlib
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))

from memehub.github import ApiError
from memehub.protocol import expected_manifests, read_entries

REPO = "example/images"
BASE = "1" * 40
HEAD = "2" * 40
PNG = b"\x89PNG\r\n\x1a\nfixture-image"


def entry(user=7, name="Pig", data=PNG, display="大猪猪"):
    prefix = f"data/{user}/{name}/"
    metadata = f'file = "meme.png"\ndisplay_name = "{display}"\n'.encode("utf-8")
    return {prefix + "meme.png": data, prefix + "meme.toml": metadata}


def manifests(files):
    items = read_entries(files, lambda path, limit: files[path])
    return {**files, **expected_manifests(items)}


class FakeGitHub:
    def __init__(self, base=None, head=None):
        self.commits = {}
        self.blobs = {}
        self.histories = {}
        self.parents = {}
        self.add_commit(BASE, {} if base is None else base)
        self.add_commit(HEAD, {} if head is None else head)
        self.refs = {(REPO, "main"): BASE, (REPO, "submission"): HEAD}
        self.pr = {"number": 1, "state": "open", "body": "", "author_association": "NONE",
                   "user": {"id": 7, "type": "User", "login": "contributor"},
                   "base": {"ref": "main"},
                   "head": {"sha": HEAD, "ref": "submission", "repo": {"full_name": REPO}}}
        self.ancestor = BASE
        self.statuses = []
        self.comments = []
        self.closed = []
        self.writes = []
        self.receipts = set()
        self.pull_reads = 0
        self.on_pull = None
        self.on_comment = None
        self.on_update = None
        self.write_error = None
        self.tree_override = None
        self.user_permission = "read"

    def add_commit(self, sha, files):
        self.commits[sha] = dict(files)
        self.histories[sha] = {}
        for path, data in files.items():
            blob_sha = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
            self.blobs[blob_sha] = data
            self.histories[sha][path] = sha
        return sha

    def repo(self, repo):
        return {"default_branch": "main"}

    def permission(self, repo, login):
        return self.user_permission

    def pull(self, repo, number):
        self.pull_reads += 1
        if self.on_pull:
            self.on_pull(self)
        return copy.deepcopy(self.pr)

    def open_pulls(self, repo):
        return [copy.deepcopy(self.pr)] if self.pr["state"] == "open" else []

    def ref(self, repo, branch):
        return self.refs[(repo, branch)]

    def tree(self, repo, commit):
        if self.tree_override is not None:
            return copy.deepcopy(self.tree_override)
        rows = []
        for path, data in self.commits[commit].items():
            sha = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
            rows.append({"path": path, "sha": sha, "size": len(data), "mode": "100644", "type": "blob"})
        return {"truncated": False, "tree": rows}

    def blob(self, repo, sha, limit):
        return self.blobs[sha]

    def merge_base(self, repo, base, head_repo, head):
        return self.ancestor

    def generated_file(self, repo, head, path, publisher, status_repo):
        return (status_repo, self.histories[head].get(path)) in self.receipts

    def status(self, repo, sha, state, description, context="memehub/validate"):
        self.statuses.append((repo, sha, state, description, context))
        if context == "memehub/generated":
            self.receipts.add((repo, sha))

    def comment(self, repo, number, text):
        self.comments.append(text)
        if self.on_comment:
            self.on_comment(self)

    def close(self, repo, number):
        self.closed.append(number)
        self.pr["state"] = "closed"

    def create_commit(self, repo, parent, changes, message):
        if self.write_error:
            raise ApiError(self.write_error)
        files = dict(self.commits[parent])
        for path, content in changes.items():
            if content is None:
                files.pop(path, None)
            else:
                files[path] = content
        sha = hashlib.sha1((parent + str(len(self.writes)) + message).encode()).hexdigest()
        self.add_commit(sha, files)
        history = dict(self.histories[parent])
        history.update({path: sha for path in changes})
        self.histories[sha] = history
        self.parents[sha] = parent
        self.writes.append((repo, parent, changes, message))
        return sha

    def update_ref(self, repo, branch, sha):
        if self.on_update:
            self.on_update(self)
        if self.refs[(repo, branch)] != self.parents[sha]:
            raise ApiError(422)
        self.refs[(repo, branch)] = sha
        if (repo, branch) == (self.pr["head"]["repo"]["full_name"], self.pr["head"]["ref"]):
            self.pr["head"]["sha"] = sha
