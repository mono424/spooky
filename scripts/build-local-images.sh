#!/usr/bin/env bash
# Build local Docker images for SSP and scheduler tagged with :dev.
#
# Pair with `version: dev` in sp00ky.yml so `spky dev` runs the freshly built
# images instead of pulling the published canary tags from Docker Hub.
#
# Usage:
#   ./scripts/build-local-images.sh           # build both
#   ./scripts/build-local-images.sh ssp       # only ssp
#   ./scripts/build-local-images.sh scheduler # only scheduler

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

TAG="dev"
# Build for the host platform (no `--platform`). On Apple Silicon this is
# linux/arm64 and avoids the QEMU emulation tax that `--platform linux/amd64`
# imposes (~5-10× slower Rust builds). `spky dev` no longer forces a platform
# either, so docker run picks the same arch the image was built for.
SSP_IMAGE="mono424/spooky-ssp:${TAG}"
SCHEDULER_IMAGE="mono424/spooky-scheduler:${TAG}"

target="${1:-all}"

build_ssp() {
  echo "==> Building ${SSP_IMAGE} (host platform)"
  docker build -t "${SSP_IMAGE}" -f apps/ssp/Dockerfile .
}

build_scheduler() {
  echo "==> Building ${SCHEDULER_IMAGE} (host platform)"
  docker build -t "${SCHEDULER_IMAGE}" -f apps/scheduler/Dockerfile .
}

case "$target" in
  ssp)        build_ssp ;;
  scheduler)  build_scheduler ;;
  all)        build_ssp; build_scheduler ;;
  *)
    echo "Unknown target: $target (use ssp | scheduler | all)" >&2
    exit 1
    ;;
esac

echo
echo "Done. Set 'version: dev' in your sp00ky.yml to use these images."
