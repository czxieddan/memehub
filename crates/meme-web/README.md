# meme-web

中文表情图库与公开 HTTP 代理。页面、样式、脚本和图标通过 `include_str!` 嵌入二进制，构建和运行页面无需 Node、CDN 或外部字体。

## 配置与启动

```toml
[web]
listen = "127.0.0.1:3000"
core_url = "http://127.0.0.1:3001"
github_url = "http://127.0.0.1:3002"
source_url = "https://github.com/your-org/your-project"
```

调用 `meme_web::run(config_path, shutdown_receiver).await` 启动服务。三个监听或上游配置项必填；`source_url` 可省略或留空，此时 `/source` 会明确显示尚未配置。

配置只读取 `[web]`。上游地址须使用 HTTP 或 HTTPS，不接受 URL 凭据、查询参数或片段；若带路径前缀，公开路由会附加在该前缀后。源码地址只接受 HTTP 或 HTTPS，并在 HTML 属性和文本中转义。

当关闭信号变为 `true`，或信号发送端关闭时，服务停止接受新连接并等待当前连接结束。

## 公开路由

| Web 路径 | 目标 |
| --- | --- |
| `GET /api/gallery` | core `/v1/gallery`，仅保留 `query`、`cursor`、`limit` 参数 |
| `GET /api/images/{hash}` | core `/v1/images/{hash}`，hash 须为 64 位小写十六进制 |
| `GET /api/me` | core `/v1/me` |
| `GET /api/submissions` | core `/v1/submissions` |
| `POST /api/submissions` | core `/v1/submissions`，原样转发 multipart |
| `GET /auth/github` | github `/auth/github` |
| `GET /auth/callback` | github `/auth/callback`，保留 OAuth 查询参数 |
| `POST /auth/logout` | github `/auth/logout` |
| `POST /webhooks/github` | github `/webhooks/github`，保留原始字节与签名、投递和事件头 |

`/` 提供页面，`/assets/` 下只提供明确注册的嵌入资源，`GET /health/live` 返回 `{"status":"ok"}`，`GET /source` 提供源码地址说明。其余路径不会代理至上游。

普通请求只转发 Cookie、Origin、X-CSRF-Token 和 Content-Type。Webhook 只转发明确列出的 GitHub 元数据头及 Content-Type。客户端 Authorization、内部凭据及逐跳头不会转发。上游状态、Set-Cookie、Location、Content-Type、Cache-Control、X-Content-Type-Options 会保留。

代理不会跟随重定向。认证跳转允许本站相对路径、与请求 Host 一致的 HTTP(S) 地址及 `https://github.com`；其他跳转返回 502。反向代理须保留公开站点的 Host，GitHub 服务的公开站点配置也应指向该地址。

请求体上限为 25 MiB；上游响应上限为 8 MiB，图片响应上限为 5 MiB。响应逐块计数，无法通过省略 Content-Length 绕过。连接超时为 3 秒，上游请求及读取请求体的超时分别为 30 秒。

## 页面行为

图库支持名称搜索、游标分页和原图查看。页面启动时读取一次 `/api/me`，登录后从该响应取得 CSRF Token。投稿每次提交一张图片，包含 `name`、`display_name`、`file` 和 `consent=true`；客户端检查 PNG、JPEG、GIF、WebP 格式及 5 MiB 大小，最终校验由 core 服务负责。

「我的投稿」显示服务器返回的记录、处理状态、错误和 GitHub PR 链接。页面提供手动刷新；未知状态会显示原始值。创建时间支持 Unix 秒和日期字符串。所有内容均来自服务响应，资源文件不含示例表情数据。

原生表单、对话框和按钮提供键盘操作，标签页支持方向键及 Home/End。样式包含可见焦点和减少动画设置。

## 验证

```sh
cargo test -p meme-web
```

测试使用本地 Axum 或 TCP 上游，覆盖路由限制、参数和头过滤、multipart 与 Webhook 字节转发、重定向、体积限制、错误以及源码配置校验。测试运行无需访问公共网络。
