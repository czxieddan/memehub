# 工具、镜像与远程服务依赖

Rust 包的逐项版本、内容摘要、版权人及许可副本见 [AHCL-DEPENDENCIES.md](../.ahcl/AHCL-DEPENDENCIES.md)。此表补充构建、部署、测试和发布流程依赖。版本受 `rust-toolchain.toml`、Dockerfile、工作流和具体构建记录约束。

| 项目 / 权利人 | 用途与获取方式 | 版本或约束 | 许可 / 材料 |
| --- | --- | --- | --- |
| Rust / Rust Project Developers | Cargo、rustc、rustfmt、Clippy；通过 <https://rustup.rs> 获取 | 1.95.0，`rust-toolchain.toml` | MIT OR Apache-2.0；<https://github.com/rust-lang/rust/tree/1.95.0> 中的 LICENSE-MIT、LICENSE-APACHE、COPYRIGHT |
| Python / Python Software Foundation | 仓库脚本、许可清单与离线测试；<https://www.python.org/downloads/> | Python 3.11+，本地验证 3.12 | PSF-2.0；<https://docs.python.org/3/license.html> |
| Git / Git contributors | Cargo Git 依赖、源码版本与 PR 生成提交；<https://git-scm.com/downloads> | Git 2.x | GPL-2.0-only；<https://github.com/git/git/blob/master/COPYING> |
| Docker Engine、Compose、BuildKit / Docker 与 Moby contributors | Dockerfile、Compose 运行；<https://docs.docker.com/engine/install/> | Docker Engine 29.x、Compose v2 | Apache-2.0；<https://github.com/moby/moby/blob/master/LICENSE>、<https://github.com/docker/compose/blob/main/LICENSE>、<https://github.com/moby/buildkit/blob/master/LICENSE> |
| rust official image / Docker Official Images contributors | 容器构建基础镜像；`docker pull rust:1.95-bookworm` | Rust 1.95 / Debian bookworm | <https://github.com/rust-lang/docker-rust>；镜像内 Rust 与 Debian 软件许可 |
| Debian / Debian contributors 与各软件包权利人 | 运行时基础镜像；`docker pull debian:bookworm-slim`；ca-certificates 来自 Debian apt | Debian 12 bookworm；构建记录中的 digest 与 apt 版本 | 各软件包许可位于镜像 `/usr/share/doc/*/copyright`；<https://www.debian.org/legal/licenses/> |
| GitHub API / GitHub, Inc. | OAuth、GitHub App、仓库内容、PR 与 webhook 远程服务；<https://docs.github.com/rest> | REST API，GitHub.com 当前服务 | GitHub 服务条款；<https://docs.github.com/site-policy/github-terms/github-terms-of-service>；服务实现不随本项目分发 |
| actions/checkout / GitHub | 受信任基础分支检出；GitHub Actions 获取 | 工作流固定 commit | MIT；<https://github.com/actions/checkout/blob/11bd71901bbe5b1630ceea73d27597364c9af683/LICENSE> |
| actions/setup-python / GitHub | 数据仓库 CI 提供 Python | 模板固定 commit | MIT；<https://github.com/actions/setup-python/blob/a26af69be951a213d495a4c3e4e4022e16d87065/LICENSE> |
| actions/create-github-app-token / GitHub | 数据仓库发布 App 安装令牌 | 模板固定版本 | MIT；<https://github.com/actions/create-github-app-token/blob/main/LICENSE> |

部署产物的镜像 digest、系统包清单、构建参数和应用 commit 应随实际发布记录保留。本表中的镜像标签描述可用的基础版本，具体构建日志提供实际拉取摘要。源码中的第三方许可副本位于 `.ahcl/THIRD-PARTY-LICENSES/` 及 `licenses/RNovModularDB/`。
