# Base stage with cargo-chef
FROM rust:1.83-bookworm as chef
RUN rustup default nightly
RUN cargo install cargo-chef

# Build dependencies for protobuf, SSL, and RocksDB (clang)
RUN apt-get update && apt-get install -y \
    protobuf-compiler \
    cmake \
    pkg-config \
    libssl-dev \
    clang \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

# Planner stage
FROM chef as planner
COPY apps/scheduler apps/scheduler
WORKDIR /usr/src/app/apps/scheduler
RUN cargo chef prepare --recipe-path recipe.json

# Cacher stage
FROM chef as cacher
WORKDIR /usr/src/app
COPY --from=planner /usr/src/app/apps/scheduler/recipe.json apps/scheduler/recipe.json
WORKDIR /usr/src/app/apps/scheduler
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo chef cook --release --recipe-path recipe.json

# Builder stage
FROM chef as builder
COPY apps/scheduler apps/scheduler
COPY --from=cacher /usr/src/app/apps/scheduler/target apps/scheduler/target
COPY --from=cacher /usr/local/cargo /usr/local/cargo
WORKDIR /usr/src/app/apps/scheduler
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create directories for replica DB and WAL
RUN mkdir -p /data/replica

COPY --from=builder /usr/src/app/apps/scheduler/target/release/scheduler /usr/local/bin/

ENV RUST_LOG=info

EXPOSE 9667

CMD ["scheduler"]
