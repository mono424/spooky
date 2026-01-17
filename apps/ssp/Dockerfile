# Build Stage
FROM rust:1.83-bookworm as builder

# Switch to nightly to support edition 2024 dependencies
RUN rustup default nightly

WORKDIR /usr/src/app

# Copy the necessary local packages source
# We need to preserve the directory structure as specified in Cargo.toml dependencies (../../packages/...)
COPY packages/ssp packages/ssp

# Copy the application source
COPY apps/ssp apps/ssp

# Build the application
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
