# Source Code, History, and Licenses

The corresponding memehub source and complete project Git history are public at <https://github.com/czxieddan/memehub>. Retrieve all public history with:

```sh
git clone https://github.com/czxieddan/memehub.git
cd memehub
git checkout <image-source-revision>
cargo build --locked --release
```

`README.md` documents initialization, configuration, GitHub App permissions, deployment and verification. `docs/api.md` and `docs/architecture.md` define service interfaces and repository publication protocols. `Dockerfile` and `compose.yml` provide the container build and runtime parameters. Place real credentials only in ignored local files or deployment volumes.

The `org.opencontainers.image.revision` label identifies the application revision actually built. `Cargo.lock` fixes Rust dependency versions and registry digests; `Cargo.toml` fixes RNovModularDB at `018eaa0fbe8caf779ee6c90c8343c97da8b281e5`. Upstream source and its history are available at <https://github.com/czxieddan/RNovModularDB>. Our adapter does not modify upstream.

The root `LICENSE` and `.ahcl/` directory retain the memehub license layout. Upstream's original `LICENSE` and `.ahcl/` are preserved under `licenses/RNovModularDB/`; Docker preserves those layouts under `/app/`. Third-party evidence and license copies are indexed by `AHCL-DEPENDENCIES.md` in this directory.

Builds under development are identified by their actual public commits. The latest source is the public main branch; it may differ from a running deployment. Deployment operators must point the source entry to the corresponding public revision and retain its history.
