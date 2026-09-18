#!/usr/bin/env bash

set -euo pipefail

# Each entry is: base-image,tag1,tag2,...
# DHI (Docker Hardened Images) require authentication to pull, so they
# are excluded when login is unavailable (e.g., PRs from forks).
images=(
  "alpine:3.23,alpine3.23,alpine"
  "alpine:3.22,alpine3.22"
  "debian:trixie-slim,trixie-slim,debian-slim"
  "buildpack-deps:trixie,trixie,debian"
  "python:3.15-rc-alpine3.23,python3.15-rc-alpine3.23,python3.15-rc-alpine"
  "python:3.14-alpine3.23,python3.14-alpine3.23,python3.14-alpine"
  "python:3.13-alpine3.23,python3.13-alpine3.23,python3.13-alpine"
  "python:3.12-alpine3.23,python3.12-alpine3.23,python3.12-alpine"
  "python:3.11-alpine3.23,python3.11-alpine3.23,python3.11-alpine"
  "python:3.10-alpine3.23,python3.10-alpine3.23,python3.10-alpine"
  "python:3.9-alpine3.22,python3.9-alpine3.22,python3.9-alpine"
  "python:3.15-rc-trixie,python3.15-rc-trixie"
  "python:3.14-trixie,python3.14-trixie"
  "python:3.13-trixie,python3.13-trixie"
  "python:3.12-trixie,python3.12-trixie"
  "python:3.11-trixie,python3.11-trixie"
  "python:3.10-trixie,python3.10-trixie"
  "python:3.9-trixie,python3.9-trixie"
  "python:3.15-rc-slim-trixie,python3.15-rc-trixie-slim"
  "python:3.14-slim-trixie,python3.14-trixie-slim"
  "python:3.13-slim-trixie,python3.13-trixie-slim"
  "python:3.12-slim-trixie,python3.12-trixie-slim"
  "python:3.11-slim-trixie,python3.11-trixie-slim"
  "python:3.10-slim-trixie,python3.10-trixie-slim"
  "python:3.9-slim-trixie,python3.9-trixie-slim"
)

if [ "${LOGIN}" == "true" ]; then
  images+=(
    "dhi.io/alpine-base:3.23,alpine3.23-dhi,alpine-dhi"
    "dhi.io/debian-base:trixie-debian13,trixie-dhi,debian-dhi"
    "dhi.io/python:3.14,python3.14-dhi"
    "dhi.io/python:3.13,python3.13-dhi"
    "dhi.io/python:3.12,python3.12-dhi"
    "dhi.io/python:3.11,python3.11-dhi"
    "dhi.io/python:3.10,python3.10-dhi"
  )
fi

json=$(printf '%s\n' "${images[@]}" | jq -R . | jq -sc '{"image-mapping": .}')
echo "matrix=${json}" >> "$GITHUB_OUTPUT"
