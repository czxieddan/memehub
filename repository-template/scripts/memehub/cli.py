"""Workflow entry points. Credentials and event paths arrive through environment."""

import argparse
import json
import os
from pathlib import Path
import sys

from .github import GitHub
from .protocol import Invalid
from .publish import publish
from .pull_request import Config, Validator


def event_data():
    path = Path(os.environ["GITHUB_EVENT_PATH"])
    if path.stat().st_size > 1024 * 1024:
        raise Invalid("workflow 事件超过 1 MiB")
    return json.loads(path.read_text(encoding="utf-8"))


def configuration():
    return Config(bot_login=os.environ.get("MEMEHUB_BOT_LOGIN", ""),
                  publisher_login=os.environ.get("MEMEHUB_PUBLISHER_LOGIN", "github-actions[bot]"),
                  allow_maintainer_import=os.environ.get("MEMEHUB_ALLOW_MAINTAINER_IMPORT", "false") == "true")


def clients():
    api = GitHub(os.environ.get("MEMEHUB_TOKEN", ""))
    token = os.environ.get("MEMEHUB_HEAD_TOKEN", "")
    head = GitHub(token, api.budget) if token else None
    return api, head


def revalidate(api, repository, config):
    validator = Validator(api, repository, config)
    branch = api.repo(repository)["default_branch"]
    pulls = [pull for pull in api.open_pulls(repository) if pull["base"]["ref"] == branch]
    # Invalidate every old success before processing the queue against the new base.
    for pull in pulls:
        api.status(repository, pull["head"]["sha"], "pending", "默认分支已更新，必须重新校验")
    failed = False
    for pull in pulls:
        try:
            validator.run(pull["number"])
        except Invalid as error:
            print(f"PR #{pull['number']}: {error}", file=sys.stderr)
            failed = True
    if failed:
        raise Invalid("部分待合并 PR 未通过最新默认分支校验；请查看对应 PR 评论与状态")


def execute(mode):
    api, head_api = clients()
    repository = os.environ["GITHUB_REPOSITORY"]
    event = event_data()
    if mode == "publish":
        if event.get("pull_request", {}).get("merged") is not True:
            raise Invalid("仅允许 PR 合并事件发布 index")
        sha = publish(api, repository)
        print(f"index 已发布或已为最新：{sha}")
    elif mode == "validate":
        pull = event["pull_request"]
        Validator(api, repository, configuration(), head_api).run(pull["number"])
    else:
        revalidate(api, repository, configuration())


def main():
    parser = argparse.ArgumentParser(description="memehub 图片仓库自动化")
    parser.add_argument("mode", choices=("validate", "revalidate", "publish"))
    args = parser.parse_args()
    try:
        execute(args.mode)
    except (Invalid, KeyError, OSError, ValueError) as error:
        if isinstance(error, Invalid):
            print(str(error), file=sys.stderr)
        else:
            print("配置或事件数据无效；未输出原始响应或环境变量", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
