# memehub

GitHub 驱动的表情包分享站。用户授权 GitHub App 后上传图片，App 创建带 `Co-authored-by` 的 PR；合并并发布索引后，网站同步显示。数据库使用 RNovModularDB，只保存业务索引、身份、会话与任务状态。

- **PR 创建或更新**：Action 计算图片原始字节的 SHA-256，在 **PR 源分支**生成 `/manifest/<图片哈希>/manifest.toml`。
- **PR 合并**：另一 Action 在 **主分支**生成 `/manifest/index.toml`。
- 图片哈希全局唯一；发生碰撞时评论原因并关闭整条 PR。同一条目的正常更新按替换后的仓库状态校验。

完整协议见 [架构文档](docs/architecture.md)，数据仓库的脚本和工作流位于 [repository-template](repository-template/README.md)。源码仓库为 <https://github.com/czxieddan/memehub>。

## 本地启动

需要 Rust 1.95 和 Git。首次拉取依赖需要网络，业务测试使用本地临时数据。

```sh
cargo build --locked
cargo run --locked -- admin init
cargo run --locked -- admin config set github.app_id YOUR_APP_ID
cargo run --locked -- admin config set github.client_id YOUR_CLIENT_ID
cargo run --locked -- admin config set github.installation_id 123456
cargo run --locked -- admin config set github.allowed_repositories '["OWNER/memes"]'
cargo run --locked -- admin config set core.repository OWNER/memes
cargo run --locked -- admin secret github-app --file /path/to/app.pem
cargo run --locked -- admin secret github-client --file /path/to/client-secret.txt
cargo run --locked -- serve
```

访问 <http://localhost:7410>。`admin init` 在 `config/local.toml` 和 `.local/secrets/` 生成配置、独立服务凭据和数据库页密钥；已有文件会使初始化失败。密钥目录不要入库。备份数据库时也要安全备份 `database-key`，丢失后无法解密。

## GitHub App

1. 创建 GitHub App，启用用户授权和 **Request user authorization (OAuth) during installation** 所需的 Web flow；设置回调 `https://YOUR_HOST/auth/callback`，webhook `https://YOUR_HOST/webhooks/github`。
2. Repository permissions：Contents **read/write**、Pull requests **read/write**、Metadata **read**；用户邮箱权限 **read**。订阅 `pull_request`、`push`、`workflow_run`、`installation`、`github_app_authorization`。
3. 安装到数据仓库，填入 App ID、Client ID、installation ID，导入 PEM 和 client secret。将 `.local/secrets/webhook-secret` 的值填到 App webhook secret。
4. 将 `github.public_base`、`core.public_base` 设为同一个对外 HTTPS 地址，保留浏览器传入的 Origin。生产通过 HTTPS 反向代理访问网页端口。
5. 把模板内容复制到数据仓库根目录，按模板说明配置发布 App 与分支保护。应用源码与数据仓库可以独立维护。

登录需要 GitHub 返回已验证邮箱；保存数值 ID、用户名、验证邮箱与贡献署名地址。私密邮箱使用 GitHub 的 noreply 署名地址。网页投稿须明确同意公开投稿与共同作者署名。首次运行没有有效 index 时图库为空，原有有效发布不会被失败同步覆盖。

## Docker

```sh
docker compose build
docker compose run --rm app admin init
docker compose run --rm app admin config set web.listen 0.0.0.0:7410
# 按上面说明用 admin config set 和 admin secret 配置 GitHub App
# 导入文件也可通过 stdin：cat /path/to/app.pem | docker compose run --rm -T app admin secret github-app
docker compose up -d
```

Compose 仅把 `127.0.0.1:7410` 映射到宿主机；配置和数据库/暂存文件保存在两个命名卷。其他服务监听容器内回环地址，API 使用不同凭据。容器以非 root 用户运行，根文件系统只读。构建时设置 `SOURCE_REVISION` 为已公开的 Git commit，并保留镜像的源码地址与版本标签。

## 管理

```sh
memehub admin cache show
memehub admin cache set --max-bytes 134217728 --max-entries 5000 --ttl-seconds 900
memehub admin sync
memehub admin submissions
memehub admin pulls
memehub admin retry <submission-uuid>
memehub admin backup
```

缓存设置保存到数据库并即时应用，降低上限或 TTL 会立即清理。缓存按衰减访问热度淘汰。`admin config set` 的字符串直接传值，数组用 TOML 表达式；服务配置修改后需重启，缓存设置无需重启。`admin data` 提供命名空间通用记录管理。

## 代码与检查

五个业务 crate 不互相依赖，通过 [HTTP API](docs/api.md) 通信；根入口负责装配。可以用 `serve --service database|github|core|web` 单独启动，需自行保证数据库先就绪。

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python -m unittest discover -s repository-template/tests -v
docker compose config --quiet
```

当前支持 PNG/JPEG/GIF/WebP，单张默认 5 MiB，单帧像素默认上限 16,777,216。`Name` 为 1–64 位 ASCII 字母、数字、下划线或连字符；外显名称最多 120 个 Unicode 字符。一个网页投稿对应一个条目和一个 PR；发布后修改请通过仓库 PR。

## 许可与源码

项目采用 AHCL 1.2，正文与上游材料见 [LICENSE](LICENSE)、[NOTICE.md](NOTICE.md) 和 [licenses](licenses/)。第三方依赖保留各自许可。公开构建应能够对应到公开 Git 历史；本项目不公开部署密钥、用户数据库或暂存上传。
