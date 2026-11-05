FROM rust:1.94-bookworm AS builder
WORKDIR /source
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

FROM builder AS api-builder
RUN cargo build --locked --release --package store-api

FROM builder AS connector-builder
RUN cargo build --locked --release --package store-connector

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && groupadd --system store && useradd --system --gid store --home-dir /app store \
    && mkdir -p /app/data /app/releases && chown -R store:store /app
USER store
WORKDIR /app

FROM runtime AS store-connector
COPY --from=connector-builder /source/target/release/store-connector /usr/local/bin/store-connector
EXPOSE 8100
ENTRYPOINT ["store-connector"]
CMD ["/app/connector.json"]

FROM runtime AS store-api
COPY --from=api-builder /source/target/release/store-api /usr/local/bin/store-api
ENV STORE_API_DATABASE=/app/data/registry.sqlite3 STORE_API_CONNECTOR_RELEASE_ROOT=/app/releases
EXPOSE 8200
CMD ["store-api"]
