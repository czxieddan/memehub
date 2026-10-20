# HTTP/JSON API v1

业务 crate 之间没有 Cargo 依赖。以下是跨服务契约；公共网页路径由 `meme-web` 显式映射，内部管理路径不会代理到外网。服务都提供 `GET /health/live`。

## modular-db-api

除健康检查外，使用 `Authorization: Bearer <数据库 principal token>`。principal 限定命名空间；admin 可以跨命名空间及备份。数据库不认识任何业务模型。记录最大 64 KiB，默认每页 100、最多 500；游标为上一条 key，按 key 排序。

| 方法与路径 | 内容 |
| --- | --- |
| `GET /v1/namespaces/{namespace}/records?cursor=&limit=&prefix=` | `{items:[{key,version,value}],next_cursor}` |
| `GET /v1/namespaces/{namespace}/records/{key}` | `{key,version,value}`；不存在为 404 |
| `PUT /v1/namespaces/{namespace}/records/{key}` | `{value,expected_version?}` |
| `DELETE /v1/namespaces/{namespace}/records/{key}?expected_version=` | 条件删除 |
| `GET /v1/namespaces/{namespace}/indexes` | 索引定义 `{items:[{name,pointer}]}` |
| `PUT /v1/namespaces/{namespace}/indexes/{name}` | `{pointer:"/字段/子字段"}`，事务重建索引 |
| `DELETE /v1/namespaces/{namespace}/indexes/{name}` | 删除索引定义和索引项 |
| `POST /v1/namespaces/{namespace}/indexes/{name}/query` | `{value:标量,cursor?,limit?}`，返回记录分页 |
| `POST /v1/transactions` | `{operations:[...]}`，1–10000 个，全部成功或全部回滚 |
| `POST /v1/admin/backups` | 创建加密数据库备份，返回 `{backup:"随机名称.rnmdb"}` |

事务操作：`{op:"put",namespace,key,value,expected_version?}`、`{op:"delete",namespace,key,expected_version?}`、`{op:"check",namespace,key,expected_version}`。省略版本为无条件，0 表示当前不存在，非零版本必须精确相等，冲突为 409。删除保留内部版本墓碑，重建不复用旧版本。错误为 `{error:文本}`。

标量索引由调用者声明 JSON Pointer；字段为字符串、数字、布尔值或 null 时纳入索引，缺失字段、数组和对象不纳入。查询按 JSON 类型与序列化值精确匹配，`1`、`1.0` 与 `"1"` 互不相等；结果按记录 key 排序，分页规则与记录列表相同。每个命名空间最多 32 个索引，创建或修改时最多回填 100000 条记录；索引名遵循命名空间命名规则。记录更新、删除与索引维护在同一事务中；索引继承命名空间权限。

## meme-github

`/v1/*` 使用独立的 GitHub 服务 token。用户会话通过 HttpOnly、SameSite=Lax cookie `memehub_session` 传递；HTTPS 配置下添加 Secure。

身份和会话保存同一轮授权编号。普通再次登录保留有效会话；撤销授权使本轮会话全部失效，重新授权不会恢复旧会话。身份与新会话以条件事务写入，遇到并发身份变更时拒绝本次登录。没有授权编号的旧格式会话需重新登录。

| 方法与路径 | 内容 |
| --- | --- |
| `GET /auth/github` | 保存一次性 OAuth state、PKCE verifier，跳转 GitHub |
| `GET /auth/callback?code=&state=` | 校验 state、交换 token、保存已验证身份，建立本地会话 |
| `POST /auth/logout` | 校验 Origin 和 `X-CSRF-Token`，删除会话 |
| `POST /webhooks/github` | 原始请求体的 `X-Hub-Signature-256` 验签，按 delivery ID 去重 |
| `GET /v1/session` | 有效身份 `{id,login,email,contribution_email,csrf_token}`；无效为 401 |
| `POST /v1/changes` | 创建 App 分支和 PR，见下例 |
| `GET /v1/pulls/{number}?repository=OWNER/REPO` | PR 状态，含 `state`、`merged`、`url` |
| `GET /v1/pulls?repository=OWNER/REPO&page=1` | `{items,next_page}`，用于周期对账 |
| `GET /v1/content?repository=&path=&reference=` | `{content_base64}`；不存在为 404 |
| `GET /v1/events?cursor=` | 持久事件 `{items:[{cursor,event}],next_cursor}` |

`changes` 请求含 `operation_id`（UUID 幂等键）、`repository`、`base_branch`、`title`、`body`、`message`、可选 `coauthor_user_id`、`files:[{path,content_base64}]`。允许的仓库和文件前缀来自配置。署名必须从已保存的授权用户信息生成。返回 `{operation_id,repository,number,url,branch}`。这里的 Base64 仅用于服务间传输，不存数据库。

## meme-core

| 方法与路径 | 内容 |
| --- | --- |
| `GET /v1/gallery?query=&cursor=&limit=` | `{items:[{manifest_hash,uploader_id,name,display_name}],next_cursor}`；默认每页 40 |
| `GET /v1/images/{hash}` | 已发布图片，带正确 Content-Type 与 nosniff |
| `GET /v1/me` | 会话身份；无效为 401 |
| `GET /v1/submissions` | 当前用户的投稿状态列表 |
| `POST /v1/submissions` | multipart：`name`、`display_name`、`file`、`consent=true`；必须有效 cookie、Origin 和 `X-CSRF-Token` |
| `GET /v1/admin/cache` | 当前策略、占用和统计 |
| `PUT /v1/admin/cache` | `{max_bytes,max_entries,ttl_seconds}` 持久化并即时应用 |
| `POST /v1/admin/sync` | 拉取仓库索引，验证通过后事务切换 |
| `GET /v1/admin/submissions` | 管理投稿任务 |
| `GET /v1/admin/pulls` | 查询外部及网页 PR 的对账记录 |
| `POST /v1/admin/submissions/{id}/retry` | 重新执行创建 PR 失败的任务 |

所有 `/v1/admin/*` 使用 core 管理 token。投稿状态依次为 `pending`、`retrying`、`pr_open`、`awaiting_publish`、`published`；也可进入 `failed` 或 `closed`。`merged=true` 不直接代表发布完成，必须等待有效 index 包含该条目。

## meme-web

| 公共路径 | 内部目标 |
| --- | --- |
| `GET /api/gallery` | core `/v1/gallery` |
| `GET /api/images/{hash}` | core `/v1/images/{hash}` |
| `GET /api/me` | core `/v1/me` |
| `GET/POST /api/submissions` | core `/v1/submissions` |
| `/auth/github`、`/auth/callback`、`/auth/logout` | GitHub 同名路由 |
| `POST /webhooks/github` | GitHub 同名路由，保持原始请求体 |
| `GET /source` | 源码、许可及对应版本说明页面 |

未知路径返回 404，错误方法返回 405；公共代理不接收任意上游 URL，不转发浏览器的 Authorization、Host 或转发身份头。实际服务端凭据保留在各服务内部。
