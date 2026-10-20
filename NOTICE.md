# 版权、上游与对应源码

memehub 的维护者为 GitHub 用户 [czxieddan](https://github.com/czxieddan)。本项目按 AHCL 1.2 公开源码路线发布；许可正文位于 `.ahcl/AHCL-1.2.md`。第三方代码和素材保留其许可。

## RNovModularDB

- 上游：<https://github.com/czxieddan/RNovModularDB>
- 固定源码版本：`018eaa0fbe8caf779ee6c90c8343c97da8b281e5`
- 直接使用：`rnmdb-cli`、`rnmdb-storage`、`rnmdb-types`，由 `modular-db-api` 适配为通用 HTTP 记录及事务接口。
- 本项目没有修改上游；完整许可、通知、依赖及相关材料原样保留在 `licenses/RNovModularDB/`。该目录材料中的相对路径应在该上游材料目录内解析。

## 源码映射

- 完整应用源码和 Git 历史：<https://github.com/czxieddan/memehub>
- 版本 `0.1.0` 对应公开仓库的 `Cargo.toml`；具体构建以公开 Git commit 为准。
- 数据库源码由工作区中的固定 Git revision 对应，依赖版本由 `Cargo.lock` 固定。
- Docker 构建使用 `--build-arg SOURCE_REVISION=<公开 commit>`，镜像标签记录应用源码地址、commit 与许可。
- 每次构建、测试或部署前，先将对应源码及必要构建材料推送到公开仓库；持续公开该版本的 Git 历史。修改后的构建不得继续标记为未修改的旧 commit。

部署时网页 `/source` 提供源码入口。该入口由 `web.source_url` 配置，默认指向本项目；部署修改版应指向公开对应源码及版本历史。

应用的公开源码不包含 GitHub 私钥、OAuth secret、数据库页密钥、会话、用户邮箱或暂存图片。表情包的数据许可由数据仓库单独声明，GitHub OAuth 与共同作者同意不能代替图片权利授权。
