# Source Code, History, and Licenses

The corresponding memehub source and complete project Git history are public at <https://github.com/czxieddan/memehub>. Retrieve all public history with:

```sh
git clone https://github.com/czxieddan/memehub.git
cd memehub
git checkout <image-source-revision>
cargo build --locked --release
```

`README.md` documents initialization, configuration, GitHub App permissions, and deployment. `docs/api.md` defines service interfaces and repository publication protocols. `Dockerfile` and `compose.yml` provide container build and runtime parameters. Place real credentials only in ignored local files or deployment volumes.

The `org.opencontainers.image.revision` label identifies the application revision actually built. `Cargo.lock` fixes Rust dependency versions and registry digests; `Cargo.toml` fixes RNovModularDB at `018eaa0fbe8caf779ee6c90c8343c97da8b281e5`. Upstream source and its history are available at <https://github.com/czxieddan/RNovModularDB>. Our adapter does not modify upstream.

The root `LICENSE` and `.ahcl/` directory retain the memehub license materials. `RNovModularDB.md` in this directory preserves the pinned upstream notice and links to its source, materials, and history. AHCL Kit maintains the dependency inventory and license copies in this directory. Docker includes these materials under `/app/`.

Builds under development are identified by their actual public commits. The latest source is the public main branch; it may differ from a running deployment. Deployment operators must point the source entry to the corresponding public revision and retain its history.

<!-- BEGIN AHCL KIT MANAGED SCOPE: the-memehub-source-code-repository-automation-web-assets-and-project-maintained-documentation-third-party-dependencies-retain-their-own-licenses -->
# AHCL Source and History

- Canonical repository: <https://github.com/czxieddan/memehub>
- Canonical branch: `main`
- Covered scope: the memehub source code, repository automation, web assets and project-maintained documentation; third-party dependencies retain their own licenses

Public Source Code, Complete Modification History, build materials, and release mappings are available from the canonical repository above.
<!-- END AHCL KIT MANAGED SCOPE: the-memehub-source-code-repository-automation-web-assets-and-project-maintained-documentation-third-party-dependencies-retain-their-own-licenses -->
