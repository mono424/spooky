# Base stage with cargo-chef
FROM rust:1.83-bookworm as chef
RUN rustup default nightly
RUN cargo install cargo-chef

# --- NEU: Wichtige Build-Dependencies für OpenTelemetry (protoc) und SSL ---
RUN apt-get update && apt-get install -y \
    protobuf-compiler \
    cmake \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*
# --------------------------------------------------------------------------

WORKDIR /usr/src/app

# Planner stage
FROM chef as planner
COPY packages/ssp packages/ssp
COPY packages/ssp-protocol packages/ssp-protocol
COPY packages/job-runner packages/job-runner
COPY apps/ssp apps/ssp
WORKDIR /usr/src/app/apps/ssp
RUN cargo chef prepare --recipe-path recipe.json

# Cacher stage
FROM chef as cacher
WORKDIR /usr/src/app
COPY --from=planner /usr/src/app/apps/ssp/recipe.json apps/ssp/recipe.json
COPY packages/ssp packages/ssp
COPY packages/ssp-protocol packages/ssp-protocol
COPY packages/job-runner packages/job-runner
WORKDIR /usr/src/app/apps/ssp
# Cook dependencies with cache mounts - protoc wird hier gebraucht!
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo chef cook --release --recipe-path recipe.json

# Builder stage
FROM chef as builder
COPY packages/ssp packages/ssp
COPY packages/ssp-protocol packages/ssp-protocol
COPY packages/job-runner packages/job-runner
COPY apps/ssp apps/ssp
COPY --from=cacher /usr/src/app/apps/ssp/target apps/ssp/target
COPY --from=cacher /usr/local/cargo /usr/local/cargo
WORKDIR /usr/src/app/apps/ssp
# Build binary with cache mounts - protoc wird hier gebraucht!
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release

# Runtime Stage
FROM debian:bookworm-slim

# Install necessary runtime dependencies
# --- NEU: libssl3 hinzugefügt für Laufzeit-Kompatibilität ---
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*
# ------------------------------------------------------------

# Create directory for persistence
RUN mkdir -p /data

# Copy the binary from builder
COPY --from=builder /usr/src/app/apps/ssp/target/release/ssp-server /usr/local/bin/

# Set defaults
ENV RUST_LOG=info
ENV SP00KY_PERSISTENCE_FILE=/data/sp00ky_state.json

# Expose port
EXPOSE 8667

# Command to run
CMD ["ssp-server"]