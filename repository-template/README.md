# memehub 公开图片仓库

这是独立图片仓库的模板。复制本目录的全部内容（包括 `.github/`）到新的公开 GitHub 仓库，保留默认分支上的脚本与工作流。自动化只依赖 Python 3.11 标准库，不需要 Rust、pip 包或 PAT。

## 投稿格式

```text
data/121408646/BigPig/meme.png
data/121408646/BigPig/meme.toml
```

`121408646` 是 GitHub 数字用户 ID，不是用户名；`BigPig` 是区分大小写的 Name，只能含 ASCII 字母、数字、下划线和连字符，长度为 1–64。文件名不能含目录分隔符。每个 ID/Name 目录必须恰有一张图片和一个 UTF-8 `meme.toml`：

```toml
file = "meme.png"
display_name = "大猪猪"
```

图片支持 PNG、JPEG（`.jpg` 或 `.jpeg`）、WebP、GIF，单张最多 5 MiB。脚本检查扩展名和文件签名，不解码图片或改变字节。图片应先通过投稿网站的图片解码与资源限制检查。外显名称支持 Unicode；修改名称或 `display_name` 不会改变图片哈希。

直接投稿时，目录 ID 必须等于 PR 作者的数字 ID；修改和删除也要校验归属。网站使用 GitHub App 在本仓库创建投稿分支和 PR，并在 PR 正文写入：

```html
<!-- memehub-uploader:121408646 -->
```

仅当 PR 作者 `type` 为 `Bot`、且 `login` 精确等于仓库变量 `MEMEHUB_BOT_LOGIN` 时，才采用这个 ID。其他作者正文中的标记不会改变归属；`Co-authored-by` 可用于署名，不能授权代投。标记必须唯一且格式完整。网站须在创建 PR 时就填写标记。

## 校验与发布协议

PR 的 opened、reopened、synchronize 事件启动校验。受信任脚本通过固定 `https://api.github.com` REST API 按 commit SHA 读取文件数据，检查最新默认分支与本 PR 的最终图片集合。全仓库以**图片原始字节的小写 SHA-256** 唯一，包括同一用户不同 Name 的图片；更新本条目时，旧版本不算另一条投稿。

发现重复后，机器人评论本次 head SHA、重复图片路径、已有图片的具体路径和完整 SHA-256，并关闭整条 PR。评论前及关闭前分别重读 head；若 head 已变，停止关闭。GitHub 关闭 PR 的接口没有原子 head 条件参数，因此最后一次查询到关闭请求之间仍存在极短的竞态窗口。

校验通过后，机器人仅在 **PR 源分支** 提交单条 manifest，并清理替换或删除条目留下的旧 manifest：

```text
manifest/<64位图片SHA-256>/manifest.toml
```

```toml
schema_version = 1
hash = "<64位小写SHA-256>"
uploader_id = 121408646
name = "BigPig"
display_name = "大猪猪"
path = "/data/121408646/BigPig/meme.png"
```

上面的占位 hash 仅用于说明字段。正式文件由脚本生成。PR 阶段不生成或改写 `manifest/index.toml`。投稿者也不能编辑 manifest、index、workflow、脚本或其他路径。机器人为自己创建的 commit 在**目标仓库**写入 `memehub/generated` 回执；重新执行时校验最近修改生成文件的 commit 及回执的 Bot 身份，识别自己的输出并保持幂等。fork 内自行创建的回执无效；请保留机器人的生成 commit，不要手工重建或 squash 后再更新投稿。

仅在 PR 合并到默认分支后，发布工作流重新获取**当前最新**默认分支，逐条验证所有图片、`meme.toml` 和已有 manifest。它只提交 `manifest/index.toml`，不会补建单条 manifest：

```toml
schema_version = 1
source_commit = "<生成index之前的40位Git提交SHA>"
hashes = ["<按字典序排列的64位图片SHA-256>"]
```

空仓库写入 `hashes = []`。读取图库时先取得 index，再固定 `source_commit` 读取 manifest 和图片，不能混用实时 main 内容。发布失败会保留旧 index。重复发布相同内容不产生提交；发布通过非强制 ref 更新避免覆盖并发提交，遇到默认分支更新最多重新读取并重试 3 次。

## GitHub App 与权限

推荐使用独立的发布 GitHub App，并安装到图片仓库。App 的仓库权限设置为 Contents、Pull requests、Commit statuses 的 Read and write，Metadata 为 Read；不授予 Actions 写权限。配置：

| 类型 | 名称 | 内容 |
| --- | --- | --- |
| Repository variable | `MEMEHUB_APP_ID` | 发布 App 的数字 App ID |
| Actions secret | `MEMEHUB_APP_PRIVATE_KEY` | App 私钥，保留原始换行 |
| Repository variable | `MEMEHUB_BOT_LOGIN` | 网站投稿 App 的精确 login，例如 `memehub-web[bot]`；留空则不接受代投标记 |
| Repository variable | `MEMEHUB_ALLOW_MAINTAINER_IMPORT` | 默认 `false`；仅字面值 `true` 开启维护者导入 |

发布者身份由 token action 的 `app-slug` 推导，网站作者 App 与发布 App 可以不同。导入开关启用后，脚本还会通过 REST 验证 PR 作者对目标仓库具有 `admin` 或 `maintain` 权限，才允许跨 ID 新增、修改、删除；普通组织成员和仅有 write 权限的协作者不会获得代投权限。导入不绕过内容、重复或生成文件规则。

未配置 App 时，同仓库投稿可使用工作流声明的 `GITHUB_TOKEN` 权限：`contents: write`、`pull-requests: write`、`statuses: write`。仓库 Actions 设置须允许这些写权限。GITHUB_TOKEN 生成的提交通常不会触发新的 workflow；脚本会主动在**生成后的 head SHA** 写入 `memehub/validate` 状态，不依赖再次触发。App 路径也始终主动写状态。

fork 投稿要自动写入 manifest，**head fork 必须安装同一发布 App**，授予 Contents/Commit statuses 写权限。工作流会单独申请该 fork 的安装令牌，仅读写该源分支；目标仓库的 GITHUB_TOKEN 不能跨仓库写入。未安装或无权限会在 PR 中给出包含实际 fork 名称的错误，例如：

```text
无法写入 PR 源仓库 alice/images：fork 必须安装同一发布 GitHub App，授予 contents/statuses 写权限，并提供该 fork 的安装令牌；不会回退写入默认分支。
```

安装 App 后重跑失败的工作流，或推送新的投稿提交。网站默认采用同仓库分支，可免去 fork 安装步骤。主分支 push 后重验既有 fork PR 不需要 fork 写令牌；如果恰逢该 PR 还有待生成变更，重验会明确失败，随后由该 PR 的 synchronize 任务取得 fork 令牌生成。

`actions/checkout`、`actions/setup-python`、`actions/create-github-app-token` 均已固定官方仓库的完整 commit；发布 token action 为 `fee1f7d63c2ff003460e3d139729b119787bc349`（v2）。升级时从官方仓库核实版本及变更，再更新固定 SHA。

## 分支保护与并发

默认分支必须启用以下规则，才能封住多个 PR 同时通过后的重复投稿竞态：

1. 必需状态为 **`memehub/validate`**，推荐限定来源为发布 App；不要误选工作流 job 的显示名称。
2. 打开 **Require branches to be up to date before merging**，要求合并前跟上最新默认分支。模板没有 `merge_group` 处理，不能只打开合并队列就关闭这个要求。
3. 限制直接推送、force push 和绕过规则的账号。发布 App 需要受控的默认分支写入/规则绕过权限以提交 index；只将该权限交给发布 App，投稿者不能绕过。
4. 维护脚本和工作流的变更由维护者独立审查，通过受控维护流程合入；普通图片投稿校验会拒绝这些路径。

默认分支 push（包括合并）会先把待合并 PR 的状态改为 pending，再对照最新默认分支逐条重验。若另一个 PR 已先合入同图，后验 PR 会被评论并关闭。GITHUB_TOKEN 导致事件不再触发时，严格的最新分支要求仍阻止旧 PR 直接合并；更新分支后会重新 synchronize。

同一 PR 的任务串行且不取消正在写入的任务；push 重验单独串行。index 发布对整个仓库的默认分支串行，`cancel-in-progress: false`；数据 API 仍检查实时 ref，并使用 `force: false` 更新。index 生成不会触发另一次合并事件，且只有 `manifest/index.toml` 变化的 push 被重验工作流忽略，避免发布循环。

## 安全边界与运行上限

特权工作流采用 `pull_request_target`，checkout 仅固定到事件提供的受信任 base commit（push 重验固定到默认分支事件 SHA），`persist-credentials: false`。绝不 checkout PR head、运行 PR 脚本、下载 PR artifact 执行、执行图片或把 PR 文本插入 shell。PR 文件保留在内存中作为数据；API URL 不接受环境变量覆盖，禁用重定向，令牌和原始服务端错误不会输出。

每次最多 10,000 个 Git 树节点、500 个条目、128 MiB 已下载 blob、256 MiB HTTP 响应总量、2,048 次 REST 请求；每个响应最多 8 MiB，单请求超时 20 秒，脚本请求预算 540 秒，job 上限 15 分钟。单张图片最多 5 MiB，描述/manifest 最多 32 KiB，index 最多 64 KiB。push 重验最多 20 个打开的 PR，超限明确失败，需减少队列后重跑。树截断、符号链接、可执行文件、子模块、路径穿越、错误文件数量均拒绝。

这些上限适合轻量仓库。扩容时应审查预算与 GitHub API 速率，不能把截断树当成完整仓库继续发布。不要把发布 App 私钥暴露给网站前端或投稿分支，也不要允许投稿者控制目标仓库中的其他特权工作流。

## 离线验证

在本目录执行，Python 3.11+ 即可：

```sh
PYTHONDONTWRITEBYTECODE=1 python -m unittest discover -s tests -v
```

测试通过内存 fake API 和临时事件文件覆盖协议、幂等生成、重复关闭与 head 复核、权限与代投、fork、删除、Unicode、并发发布以及截断树、路径和模式失败分支，不使用网络或真实 GitHub 凭据。`scripts/memehub/` 可作为受信任工作流模块执行：

```sh
PYTHONPATH=scripts python -m memehub.cli validate
PYTHONPATH=scripts python -m memehub.cli revalidate
PYTHONPATH=scripts python -m memehub.cli publish
```

这些命令会调用 GitHub 并写入分支/状态；仅供已配置凭据和事件文件的工作流使用。发布命令只接受 merged PR 事件。新仓库先合入模板，再通过第一条图片 PR 完成首次发布；在首次成功发布前没有 index，应用应将仓库视为尚未发布。
