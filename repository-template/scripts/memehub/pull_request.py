"""PR validation and source-branch-only manifest generation."""

from dataclasses import dataclass
import re

from .github import ApiError, repository_path
from .protocol import (Duplicate, Invalid, MAX_METADATA, check_manifest_path,
                       data_location, expected_manifests, unique_entries)
from .repository import changed_paths, overlay, read_tree

MARKER = re.compile(r"<!-- memehub-uploader:([1-9][0-9]{0,19}) -->")


@dataclass(frozen=True)
class Config:
    bot_login: str = ""
    publisher_login: str = "github-actions[bot]"
    allow_maintainer_import: bool = False


@dataclass
class Context:
    pull: dict
    branch: str
    base_sha: str
    ancestor: object
    base: object
    head: object


def bot_marker(body):
    if "<!-- memehub-uploader:" not in body:
        return None
    markers = MARKER.findall(body)
    if len(markers) != 1 or body.count("<!-- memehub-uploader:") != 1:
        raise Invalid("投稿机器人 PR 必须包含一个有效的 memehub-uploader 标记")
    return int(markers[0])


def uploader(pull, config):
    user = pull["user"]
    if user.get("type") == "Bot" and user.get("login") == config.bot_login:
        marker = bot_marker(pull.get("body") or "")
        if marker is not None:
            return marker
    identity = user["id"]
    if type(identity) is not int or identity <= 0:
        raise Invalid("PR 作者缺少有效的 GitHub 数字 ID")
    return identity


def maintainer_import(pull, config, api, repository):
    if not config.allow_maintainer_import:
        return False
    if pull["user"].get("type") != "User":
        return False
    return api.permission(repository, pull["user"]["login"]) in {"admin", "maintain"}


def authorize_data(pull, config, paths, api, repository):
    owner = uploader(pull, config)
    if maintainer_import(pull, config, api, repository):
        return
    for path in paths:
        if path.startswith("data/"):
            parts = data_location(path)
            if int(parts[1]) != owner:
                raise Invalid(f"投稿目录用户 ID 必须等于已核实的 PR 投稿者 {owner}：{path}")


def manifest_changes(head):
    expected = expected_manifests(head.entries())
    candidates = set(expected) | set(head.files)
    changes = {}
    for path in sorted(candidates):
        if not path.startswith("manifest/") or path == "manifest/index.toml":
            continue
        desired = expected.get(path)
        if optional_read(head, path) != desired:
            changes[path] = desired
    return changes


def optional_read(snapshot, path):
    if path not in snapshot.files:
        return None
    return snapshot.read(path, MAX_METADATA)


class Validator:
    def __init__(self, api, repository, config=None, head_api=None):
        repository_path(repository)
        self.api = api
        self.repository = repository
        self.config = config if config is not None else Config()
        self.head_api = head_api

    def run(self, number):
        pull = self.api.pull(self.repository, number)
        if pull["state"] != "open":
            return None
        sha = pull["head"]["sha"]
        self.api.status(self.repository, sha, "pending", "正在对照最新默认分支校验图片和 manifest")
        try:
            return self._validate(pull)
        except Duplicate as error:
            self._reject_duplicate(number, sha, error)
            raise
        except Invalid as error:
            self._report_failure(number, sha, str(error))
            raise

    def _load(self, pull):
        branch = self.api.repo(self.repository)["default_branch"]
        if pull["base"]["ref"] != branch:
            raise Invalid("投稿 PR 必须以默认分支为目标")
        head_repo = pull["head"].get("repo")
        if not head_repo:
            raise Invalid("PR 源仓库已删除")
        source = head_repo["full_name"]
        repository_path(source)
        if (source, pull["head"]["ref"]) == (self.repository, branch):
            raise Invalid("禁止向默认分支写入 PR manifest")
        base_sha = self.api.ref(self.repository, branch)
        ancestor_sha = self.api.merge_base(self.repository, base_sha, source, pull["head"]["sha"])
        base = read_tree(self.api, self.repository, base_sha)
        ancestor = read_tree(self.api, self.repository, ancestor_sha)
        head = read_tree(self.api, source, pull["head"]["sha"])
        return Context(pull, branch, base_sha, ancestor, base, head)

    def _input(self, context):
        pull = context.pull
        source = pull["head"]["repo"]["full_name"]
        paths = changed_paths(context.ancestor, context.head)
        for path in sorted(paths):
            if path.startswith("data/"):
                data_location(path)
            elif path.startswith("manifest/") and path != "manifest/index.toml":
                check_manifest_path(path)
                trusted = self.api.generated_file(source, pull["head"]["sha"], path,
                                                  self.config.publisher_login, self.repository)
                if not trusted:
                    raise Invalid(f"禁止投稿者编辑生成文件：{path}；仅接受发布机器人的生成回执")
            else:
                raise Invalid(f"投稿只能修改 data/；禁止修改 index、workflow、脚本或其他文件：{path}")
        authorize_data(pull, self.config, paths, self.api, self.repository)
        return paths

    def _validate(self, pull):
        context = self._load(pull)
        self._input(context)
        # A broken default branch is an infrastructure failure, not a PR duplicate.
        try:
            base_entries = context.base.validated()
        except Invalid as error:
            raise Invalid(f"默认分支内容无效，请维护者修复：{error}") from error
        merged = overlay(context.base, context.ancestor, context.head)
        entries = merged.entries()
        unique_entries(entries, preferred=base_entries.keys())
        changes = manifest_changes(context.head)
        self._current(context, pull["head"]["sha"])
        sha = self._write(context, changes)
        self._current(context, sha)
        self.api.status(self.repository, sha, "success", "图片唯一、投稿者及 manifest 校验通过；已核对最新默认分支")
        return sha

    def _current(self, context, sha):
        if not self._same_head(context.pull["number"], sha):
            raise Invalid("PR head 已变化，停止本次写入/关闭；等待最新提交重新校验")
        if self.api.ref(self.repository, context.branch) != context.base_sha:
            raise Invalid("默认分支已变化，等待 push 重新校验或重跑此任务")

    def _same_head(self, number, sha):
        pull = self.api.pull(self.repository, number)
        return pull["state"] == "open" and pull["head"]["sha"] == sha

    def _write(self, context, changes):
        pull = context.pull
        parent = pull["head"]["sha"]
        if not changes:
            return parent
        source = pull["head"]["repo"]["full_name"]
        writer = self._writer(source)
        try:
            sha = writer.create_commit(source, parent, changes, "chore(manifest): generate PR manifests")
            self._current(context, parent)
            writer.update_ref(source, pull["head"]["ref"], sha)
            self.api.status(self.repository, sha, "success", f"Generated from {parent}", "memehub/generated")
        except ApiError as error:
            if error.status in {401, 403, 404}:
                raise Invalid(self._permission_error(source)) from None
            raise
        return sha

    def _writer(self, source):
        if source == self.repository:
            return self.api
        if self.head_api is None:
            raise Invalid(self._permission_error(source))
        return self.head_api

    def _permission_error(self, source):
        return (f"无法写入 PR 源仓库 {source}：fork 必须安装同一发布 GitHub App，授予 "
                "contents/statuses 写权限，并提供该 fork 的安装令牌；不会回退写入默认分支。"
                "同仓库请检查 contents/statuses 写权限与分支规则。")

    def _reject_duplicate(self, number, sha, error):
        if not self._same_head(number, sha):
            return
        self.api.status(self.repository, sha, "failure", "重复图片，整条 PR 将关闭；具体路径和 SHA-256 见评论")
        self.api.comment(self.repository, number, f"memehub 校验拒绝本次 PR（head `{sha}`）：\n\n{error}")
        # Closing has no GitHub compare-and-swap endpoint; re-read immediately before PATCH.
        if self._same_head(number, sha):
            self.api.close(self.repository, number)

    def _report_failure(self, number, sha, reason):
        if self._same_head(number, sha):
            self.api.status(self.repository, sha, "failure", reason)
            self.api.comment(self.repository, number, f"memehub 校验失败（head `{sha}`）：\n\n{reason}")
