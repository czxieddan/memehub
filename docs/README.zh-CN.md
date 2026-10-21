# memehub

[English](../README.md)

基于 GitHub 仓库的表情包图库。用户通过 GitHub App 登录并上传图片，应用为投稿创建 PR，以共同作者记录投稿者署名。仓库 Action 校验图片、生成 manifest，并在合并后发布图库索引。

图片保留在 GitHub；RNovModularDB 保存身份、会话、投稿状态和图库元数据。前端按浏览器语言偏好显示英文、中文、日文、韩文或俄文，未匹配的语言回退英文。

## 本地运行

需要 Rust 1.95 和 Git。

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

访问 `http://localhost:7410`。初始化会创建 `config/local.toml` 和 `.local/secrets/` 下的独立凭据，拒绝覆盖已有文件。数据库备份必须同时保管数据库密钥，否则无法解密。

## 配置 GitHub App

1. OAuth callback 设置为 `https://YOUR_HOST/auth/callback`，webhook URL 设置为 `https://YOUR_HOST/webhooks/github`。
2. 仓库权限授予 Contents、Pull requests 读写和 Metadata 只读，用户权限授予邮箱只读。订阅 `pull_request`、`push`、`workflow_run`、`installation`、`github_app_authorization` 事件。
3. 将 App 安装到图片仓库，配置 App ID、Client ID、installation ID、私钥和 client secret。App 的 webhook secret 使用初始化生成的 `.local/secrets/webhook-secret`。
4. 将 `github.public_base` 和 `core.public_base` 设为同一个公网 HTTPS 地址。
5. 把 [repository-template](../repository-template/README.md) 的全部内容复制到图片仓库，包括 `.github/`，并按说明配置发布凭据和分支保护。

登录需要已验证的 GitHub 邮箱；上传前须同意公开审查和共同作者署名。首次有效索引发布前，图库为空。刷新失败时继续提供上一份有效发布。

## Docker 部署

```sh
export SOURCE_REVISION=$(git rev-parse HEAD)
docker compose build
docker compose run --rm app admin init
docker compose run --rm app admin config set web.listen 0.0.0.0:7410
# 使用前面的管理命令配置 App。
# 通过标准输入导入私钥：
cat /path/to/app.pem | docker compose run --rm -T app admin secret github-app
docker compose up -d
```

配置和数据保存在命名卷中。容器以非 root 用户运行，根文件系统只读。修改服务配置后重启：

```sh
docker compose restart app
docker compose logs --tail 100 app
```

在同一服务器上配置 HTTPS 反向代理，转发到 `http://127.0.0.1:7410`，保留原始 Host、Origin，请求体限制至少设为 6 MiB。

| 端口 | 开放范围 |
| --- | --- |
| TCP 443 | 公网 HTTPS，供图库、登录回调和 webhook 使用 |
| TCP 80 | 可选，用于 HTTP 跳转或 HTTP-01 证书验证 |
| TCP 7410 | 提供的 Compose 仅绑定本机回环地址，无需公网放行 |
| TCP 7411–7413 | 数据库、GitHub 和核心服务内部 API，无需公网放行 |

若限制出站流量，允许访问 GitHub 的 HTTPS。构建镜像还需要访问配置的镜像及软件包仓库。SSH 仅允许管理地址访问。

## 管理

Docker 部署时，通过 `docker compose exec app` 执行：

```sh
memehub admin cache show
memehub admin cache set --max-bytes 134217728 --max-entries 5000 --ttl-seconds 900
memehub admin sync
memehub admin submissions
memehub admin pulls
memehub admin retry <submission-uuid>
memehub admin backup
```

缓存策略保存到数据库并立即生效，降低上限会立即淘汰条目。配置文件中的服务设置在重启后生效。`admin data` 提供通用记录管理。

上传支持 PNG、JPEG、GIF、WebP，默认最多 5 MiB，每帧最多 16,777,216 像素。`Name` 为 1–64 个 ASCII 字母、数字、下划线或连字符；外显名称最多 120 个 Unicode 字符。每次网页上传创建一个条目和一个 PR。

## 开发

五个业务 crate 通过 [HTTP API](api.md) 通信，互相没有 Cargo 依赖。可用 `serve --service database|github|core|web` 单独启动服务，先启动数据库。

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --locked
docker compose config --quiet
```

许可证材料由 AHCL Kit 和 `.ahclkitconfigs` 维护：

```sh
ahcl config validate
ahcl project generate
ahcl project check
```

许可见 [LICENSE](../LICENSE)。
