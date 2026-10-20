# Build from repository root; the only shared input is the pure telemetry crate.
ARG RUST_IMAGE=rust:1.97.0-slim-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim
FROM ${RUST_IMAGE} AS build
WORKDIR /app/business-apps/stores/backend
ARG CARGO_BUILD_JOBS=2
COPY business-apps/stores/backend/Cargo.toml business-apps/stores/backend/Cargo.lock ./
COPY business-apps/stores/backend/src ./src
COPY business-apps/stores/backend/migrations ./migrations
COPY crates/operational-telemetry /app/crates/operational-telemetry
RUN cargo build --release --locked --bin stores-api

FROM ${RUNTIME_IMAGE}
ARG SOURCE_REVISION
ARG INPUTS_SHA256
ARG BUILD_ID
LABEL org.opencontainers.image.title="Independent Rust stores API" \
    org.opencontainers.image.revision="${SOURCE_REVISION}" \
    saas.application="stores" \
    saas.ci.inputs-sha256="${INPUTS_SHA256}" \
    saas.build-id="${BUILD_ID}"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 business \
    && useradd --system --uid 10001 --gid business --no-create-home business
COPY --from=build /app/business-apps/stores/backend/target/release/stores-api /usr/local/bin/stores-api
ENV NODE_ENV=production STORES_HOST=0.0.0.0 STORES_PORT=3031 TZ=Asia/Shanghai RUST_LOG=info
USER 10001:10001
EXPOSE 3031
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=5s --timeout=4s --start-period=30s --retries=3 \
    CMD curl --fail --silent --max-time 3 "http://127.0.0.1:${STORES_PORT}/stores-api/health/ready" > /dev/null || exit 1
ENTRYPOINT ["stores-api"]
