# syntax=docker/dockerfile:1.7
# Unified Dockerfile for ssp-server and scheduler.
# Build a specific image with: docker buildx build --target ssp ...
# or                          docker buildx build --target scheduler ...

FROM rust:1.90-bookworm AS chef
RUN apt-get update && apt-get install -y --no-install-recommends \
        protobuf-compiler \
        cmake \
        pkg-config \
        libssl-dev \
        clang \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install --locked cargo-chef
WORKDIR /usr/src/app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS cacher
COPY --from=planner /usr/src/app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo chef cook --release --recipe-path recipe.json \
        -p ssp-server -p scheduler

FROM chef AS builder
COPY . .
COPY --from=cacher /usr/src/app/target target
COPY --from=cacher /usr/local/cargo /usr/local/cargo
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release -p ssp-server -p scheduler

FROM debian:bookworm-slim AS runtime-base
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
    && rm -rf /var/lib/apt/lists/*

FROM runtime-base AS ssp
RUN mkdir -p /data
COPY --from=builder /usr/src/app/target/release/ssp-server /usr/local/bin/
ENV RUST_LOG=info \
    SP00KY_PERSISTENCE_FILE=/data/sp00ky_state.json
EXPOSE 8667
CMD ["ssp-server"]

FROM runtime-base AS scheduler
RUN apt-get update && apt-get install -y --no-install-recommends curl \
    && rm -rf /var/lib/apt/lists/*
RUN mkdir -p /data/replica
COPY --from=builder /usr/src/app/target/release/scheduler /usr/local/bin/
ENV RUST_LOG=info
EXPOSE 9667
CMD ["scheduler"]
