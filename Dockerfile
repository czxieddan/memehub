FROM rust:1.95-bookworm AS build
WORKDIR /build
COPY . .
RUN cargo build --locked --release

FROM debian:bookworm-slim
ARG SOURCE_REVISION=unknown
LABEL org.opencontainers.image.source="https://github.com/czxieddan/memehub" \
      org.opencontainers.image.revision="${SOURCE_REVISION}" \
      org.opencontainers.image.licenses="LicenseRef-AHCL-1.2"
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home memehub \
    && mkdir -p /app/config /app/.local && chown -R memehub:memehub /app
COPY --from=build /build/target/release/memehub /usr/local/bin/memehub
COPY --from=build /build/LICENSE /app/
COPY --from=build /build/.ahcl /app/.ahcl
COPY --from=build /build/config/example.toml /app/config/example.toml
USER 10001:10001
WORKDIR /app
EXPOSE 7410
ENTRYPOINT ["memehub", "--config", "/app/config/local.toml"]
CMD ["serve"]
