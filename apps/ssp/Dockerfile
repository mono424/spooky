# Base stage with cargo-chef
FROM rust:1.83-bookworm as chef
RUN rustup default nightly
RUN cargo install cargo-chef
WORKDIR /usr/src/app

# Planner stage
FROM chef as planner
COPY packages/ssp packages/ssp
COPY apps/ssp apps/ssp
# Note: cargo chef prepare needs to see the Cargo.tomls in relation to each other
# We need to compute the recipe for the specific workspace member
WORKDIR /usr/src/app/apps/ssp
RUN cargo chef prepare --recipe-path recipe.json

# Cacher stage
FROM chef as cacher
WORKDIR /usr/src/app
COPY --from=planner /usr/src/app/apps/ssp/recipe.json apps/ssp/recipe.json
# We need the dependency source manifests available for cook to work
COPY packages/ssp packages/ssp
WORKDIR /usr/src/app/apps/ssp
# Cook dependencies - this layer receives the cache
RUN cargo chef cook --release --recipe-path recipe.json

# Builder stage
FROM chef as builder
COPY packages/ssp packages/ssp
COPY apps/ssp apps/ssp
# Copy compiled dependencies from cacher
COPY --from=cacher /usr/src/app/apps/ssp/target apps/ssp/target
COPY --from=cacher /usr/local/cargo /usr/local/cargo
WORKDIR /usr/src/app/apps/ssp
RUN cargo build --release

# Runtime Stage
FROM debian:bookworm-slim

# Install necessary runtime dependencies (SSL certs, etc.)
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Create directory for persistence
RUN mkdir -p /data

# Copy the binary from builder
COPY --from=builder /usr/src/app/apps/ssp/target/release/ssp-server /usr/local/bin/

# Set defaults
ENV RUST_LOG=info
ENV SPOOKY_PERSISTENCE_FILE=/data/spooky_state.json

# Expose port
EXPOSE 8667

# Command to run
CMD ["ssp-server"]
