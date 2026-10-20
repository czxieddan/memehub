"""Bounded REST transport; no PR-controlled URL, redirect or executable content."""

import base64
import binascii
import json
import re
import time
import urllib.error
import urllib.parse
import urllib.request

from .protocol import Invalid, MAX_DATA, SHA

API_ROOT = "https://api.github.com"
MAX_RESPONSE = 8 * 1024 * 1024
MAX_REQUESTS = 2048
MAX_SECONDS = 540
REPOSITORY = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\Z")


class ApiError(Invalid):
    def __init__(self, status):
        self.status = status
        super().__init__(f"GitHub API 请求失败（HTTP {status}）；未输出令牌或服务端响应")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, *_redirect):
        raise ApiError(code)


def repository_path(repository):
    if not REPOSITORY.fullmatch(repository):
        raise Invalid("无效 GitHub owner/repository")
    if any(part in (".", "..") for part in repository.split("/")):
        raise Invalid("无效 GitHub owner/repository")
    return "/repos/" + repository


def segment(value):
    return urllib.parse.quote(str(value), safe="")


def check_route(route):
    if not route.startswith("/repos/"):
        raise Invalid("拒绝非固定 GitHub REST 路径")
    if re.search(r"[#\r\n]", route):
        raise Invalid("拒绝非固定 GitHub REST 路径")

class Budget:
    def __init__(self):
        self.started = time.monotonic()
        self.requests = 0
        self.bytes = 0

    def check(self):
        self.requests += 1
        if self.requests > MAX_REQUESTS or time.monotonic() - self.started > MAX_SECONDS:
            raise Invalid("超过单次 HTTP 请求数或 540 秒总时间限制")

    def account(self, size):
        self.bytes += size
        if self.bytes > 2 * MAX_DATA:
            raise Invalid("超过单次 256 MiB HTTP 响应总量限制")


class GitHub:
    def __init__(self, token, budget=None):
        if not token:
            raise Invalid("缺少 GitHub App token 或 GITHUB_TOKEN")
        self._token = token
        self.budget = budget if budget is not None else Budget()
        self._opener = urllib.request.build_opener(NoRedirect())
        self._blobs = {}
        self._blob_bytes = 0

    def request(self, method, route, body=None):
        check_route(route)
        self.budget.check()
        payload = None if body is None else json.dumps(body).encode("utf-8")
        headers = {"Authorization": "Bearer " + self._token,
                   "Accept": "application/vnd.github+json", "User-Agent": "memehub-template",
                   "X-GitHub-Api-Version": "2022-11-28", "Content-Type": "application/json"}
        request = urllib.request.Request(API_ROOT + route, payload, headers, method=method)
        try:
            return self._read_response(request)
        except urllib.error.HTTPError as error:
            raise ApiError(error.code) from None
        except (OSError, json.JSONDecodeError, UnicodeError):
            raise Invalid("GitHub REST 网络、超时或 JSON 响应错误") from None

    def _read_response(self, request):
        with self._opener.open(request, timeout=20) as response:
            data = response.read(MAX_RESPONSE + 1)
        self.budget.account(len(data))
        if len(data) > MAX_RESPONSE:
            raise Invalid("GitHub 单个 HTTP 响应超过 8 MiB")
        return json.loads(data)

    def repo(self, repo):
        return self.request("GET", repository_path(repo))

    def permission(self, repo, login):
        route = f"{repository_path(repo)}/collaborators/{segment(login)}/permission"
        return self.request("GET", route)["permission"]

    def pull(self, repo, number):
        return self.request("GET", f"{repository_path(repo)}/pulls/{int(number)}")

    def open_pulls(self, repo):
        result = self.request("GET", repository_path(repo) + "/pulls?state=open&per_page=21")
        if len(result) > 20:
            raise Invalid("待校验 PR 超过单次上限 20；请分批处理后重跑")
        return result

    def ref(self, repo, branch):
        result = self.request("GET", f"{repository_path(repo)}/git/ref/heads/{segment(branch)}")
        return result["object"]["sha"]

    def tree(self, repo, commit):
        if not SHA.fullmatch(commit):
            raise Invalid("树读取必须固定 40 位 commit SHA")
        return self.request("GET", f"{repository_path(repo)}/git/trees/{commit}?recursive=1")

    def blob(self, repo, sha, limit):
        if sha not in self._blobs:
            self._fetch_blob(repo, sha)
        data = self._blobs[sha]
        if len(data) > limit:
            raise Invalid("Git blob 超过文件大小限制")
        return data

    def _fetch_blob(self, repo, sha):
        if not SHA.fullmatch(sha):
            raise Invalid("无效 Git blob SHA")
        item = self.request("GET", f"{repository_path(repo)}/git/blobs/{sha}")
        if item.get("encoding") != "base64":
            raise Invalid("Git blob 不是 base64 数据")
        try:
            encoded = "".join(item["content"].split())
            data = base64.b64decode(encoded, validate=True)
        except (binascii.Error, ValueError) as error:
            raise Invalid("无效 Git blob base64") from error
        self._blob_bytes += len(data)
        if self._blob_bytes > MAX_DATA:
            raise Invalid("超过单次 128 MiB Git blob 数据限制")
        self._blobs[sha] = data

    def merge_base(self, repo, base, head_repo, head):
        owner = head_repo.split("/")[0]
        compare = segment(base) + "..." + segment(owner + ":" + head)
        result = self.request("GET", f"{repository_path(repo)}/compare/{compare}?per_page=1")
        return result["merge_base_commit"]["sha"]

    def generated_file(self, repo, head, path, publisher, status_repo):
        query = urllib.parse.urlencode({"sha": head, "path": path, "per_page": 1})
        commits = self.request("GET", repository_path(repo) + "/commits?" + query)
        if not commits:
            return False
        sha = commits[0]["sha"]
        combined = self.request("GET", f"{repository_path(status_repo)}/commits/{sha}/status?per_page=100")
        return any(trusted_receipt(status, publisher) for status in combined["statuses"])

    def status(self, repo, sha, state, description, context="memehub/validate"):
        body = {"state": state, "context": context, "description": description[:140]}
        return self.request("POST", f"{repository_path(repo)}/statuses/{sha}", body)

    def comment(self, repo, number, text):
        return self.request("POST", f"{repository_path(repo)}/issues/{int(number)}/comments", {"body": text})

    def close(self, repo, number):
        return self.request("PATCH", f"{repository_path(repo)}/pulls/{int(number)}", {"state": "closed"})

    def create_commit(self, repo, parent, changes, message):
        root = repository_path(repo)
        commit = self.request("GET", f"{root}/git/commits/{parent}")
        rows = [tree_change(path, content) for path, content in sorted(changes.items())]
        tree = self.request("POST", root + "/git/trees", {"base_tree": commit["tree"]["sha"], "tree": rows})
        result = self.request("POST", root + "/git/commits",
                              {"message": message, "tree": tree["sha"], "parents": [parent]})
        return result["sha"]

    def update_ref(self, repo, branch, sha):
        return self.request("PATCH", f"{repository_path(repo)}/git/refs/heads/{segment(branch)}",
                            {"sha": sha, "force": False})


def tree_change(path, content):
    row = {"path": path, "mode": "100644", "type": "blob"}
    if content is None:
        row["sha"] = None
    else:
        row["content"] = content.decode("utf-8")
    return row


def trusted_receipt(status, publisher):
    if status.get("context") != "memehub/generated" or status.get("state") != "success":
        return False
    creator = status.get("creator", {})
    if creator.get("type") != "Bot" or creator.get("login") != publisher:
        return False
    description = status.get("description", "")
    return bool(re.fullmatch(r"Generated from [0-9a-f]{40}", description))
