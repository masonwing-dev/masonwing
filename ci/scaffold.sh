#!/bin/sh
# CI-neutral entrypoint. Install locked dependencies with make bootstrap first.
# Product acceptance is intentionally separate and remains a required future gate.
set -eu
cd "$(dirname "$0")/.."
make check
make test
pnpm build
