# memehub 协议与架构

本文记录应用与表情包仓库共同遵守的协议，也是实现和验收的依据。

## 模块边界

Rust workspace 包含 `meme-github`、`meme-core`、`meme-cli`、`modular-db-api`、`meme-web` 五个 crate。业务 crate 之间没有 Cargo 依赖，通过版本化 HTTP/JSON API 通信。根 `src/main.rs` 只负责装配、配置路径、启动与退出。数据库层只认识命名空间、记录、条件写入和事务，不定义用户、表情包或投稿模型。

## 仓库内容

```text
/data/121408646/BigPig/meme.png
/data/121408646/BigPig/meme.toml
/manifest/<image_sha256>/manifest.toml
/manifest/index.toml
```

`meme.toml` 是投稿输入：

```toml
file = "meme.png"
display_name = "大猪猪"
```

数字目录为 GitHub 用户 ID，`BigPig` 是 Name（保留大小写，仅允许英文字母、数字、下划线和连字符）。外显名称支持 Unicode。每个 ID/Name 目录恰有一张图片及一个描述文件。图片不存数据库。

正式 manifest 的字段为 `schema_version`、`hash`、`uploader_id`、`name`、`display_name`、`path`。`path` 以 `/data/` 开头，解释为仓库根目录下的路径。哈希为图片原始字节的完整小写 SHA-256，与名称、描述文件、上传者和编码元数据无关。修改图片字节会产生新哈希，修改外显名称不改变图片哈希。

全仓库图片哈希唯一。同一用户换 Name 重复投稿同样被拒绝。更新一个条目时以替换后的仓库状态判断，不将该条目的旧版本算作重复条目。多个 PR 并发投稿时，主分支更新后必须重新验证；合并规则应要求 `memehub/validate` 成功且分支与目标分支保持最新，或使用合并队列。

## Action 流程

1. PR opened/reopened/synchronize 触发受信任的 Python 脚本。脚本读取 PR 文件作为数据，不执行 PR 内的脚本或 workflow。
2. 与目标分支及 PR 内其他图片检查哈希。碰撞时，复核 PR 最新 head SHA，评论具体原因与已有路径，并关闭整条 PR。
3. 通过后，脚本将 `/manifest/<图片哈希>/manifest.toml` 写入 PR 的源分支，删除被替换条目对应的过时 manifest。Action 不在此时更改 `/manifest/index.toml`。
4. 再运行时以仓库最终内容验证，识别自身生成结果，幂等执行。
5. PR 合并触发主分支发布 Action。它校验所有图片与现有 manifest 一致，只生成 `/manifest/index.toml` 并提交至主分支，不生成单条 manifest。
6. index 记录 `schema_version`、`source_commit`、有效 `hashes`。`source_commit` 固定一次发布的内容版本。空仓库可以发布空列表。失败时保留旧 index，网站继续使用上一份有效发布。

Python 脚本只依赖标准库。Rust crates 不生成图片哈希、manifest 或 index；CLI 也不承载这些生成命令。

网页投稿由 GitHub App 在目标仓库内创建分支及 PR。外部 fork 的源分支写入需要该 fork 给发布 App 授予相应权限；目标仓库的 GITHUB_TOKEN 本身不能写另一个仓库。没有源分支写权限时明确报错，不将 manifest 偷改写到主分支。仓库模板说明 App 配置和权限。

## 服务同步与持久化

GitHub 服务验签并去重 webhook；核心服务处理 PR 状态、主分支 push 和发布 workflow 的完成通知，并周期核对仓库，补偿漏收、乱序和重启期间的事件。只有有效 index 才驱动图库更新。读取 manifest 和图片时固定 index 的 source_commit，数据库批量切换成功后才启用该发布。

表情包索引仅包含 `manifest_hash`、`uploader_id`、`name`、`display_name`。仓库同步状态单独保存发布 commit。用户记录包含 GitHub ID、用户名、已验证邮箱及署名信息；投稿和会话使用各自的记录。图片、图片 Base64、manifest 正文不进入数据库。

暂存图片位于核心服务持久化卷中，用于可靠创建 PR，PR 创建确认后清理。图库图片与 manifest 按需读取并缓存。缓存有可配置字节数、条目数和固定 TTL；按有衰减的访问热度优先保留热门内容。更新和删除主动失效缓存。

## 管理

`memehub admin cache show` 查询当前缓存。

`memehub admin cache set --max-bytes ... --max-entries ... --ttl-seconds ...` 经管理 API 持久化并立即热更新。降低上限立即淘汰，缩短 TTL 立即清理。热度不持久化。配置、密钥、重试、同步和备份通过 CLI 管理，接口只在内部受认证入口开放。

## 访问与发布

仅前端映射宿主机端口。数据库、核心管理和 GitHub 内部接口需要独立服务凭据。OAuth 使用一次性 state 与 PKCE，投稿需要 CSRF 校验。webhook 根据原始请求体验签。上传限制、路径校验及图片资源限制同时作用于网页和仓库校验。

源码采用 AHCL 1.2 公开路线，公开仓库为 <https://github.com/czxieddan/memehub>。保留上游许可、依赖材料和发布映射，公开对应源码及版本历史；真实密钥和用户数据不公开。应用代码仓库与表情包数据仓库分开配置。
