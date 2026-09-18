#!/usr/bin/env bash

set -euo pipefail

build_directory=${1:-.}
mkdir -p "$build_directory"

# Extract the image and tags from the matrix variable.
IFS=',' read -r BASE_IMAGE BASE_TAGS <<< "${IMAGE_MAPPING}"
# The first image alias is the name of its manifest artifact.
echo "name=${BASE_TAGS%%,*}" >> "$GITHUB_OUTPUT"

# Generate the Dockerfile for this base image.
cat <<EOF > "$build_directory/Dockerfile"
FROM ${BASE_IMAGE}
COPY --from=${UV_BASE_IMAGE} /uv /uvx /usr/local/bin/
ENV UV_TOOL_BIN_DIR="/usr/local/bin"
ENTRYPOINT []
CMD ["/usr/local/bin/uv"]
EOF

# Collect Docker metadata patterns for every image alias.
TAG_PATTERNS=""

# For releases, the first tag includes the patch version and image suffix.
# Docker metadata uses it for `org.opencontainers.image.version`.
IFS=','; for TAG in ${BASE_TAGS}; do
  if [ "${PUSH_DEV}" == "true" ]; then
    TAG_PATTERNS="${TAG_PATTERNS}type=sha,suffix=-${TAG}\n"
  else
    TAG_PATTERNS="${TAG_PATTERNS}type=pep440,pattern={{ version }},suffix=-${TAG},value=${VERSION}\n"
    TAG_PATTERNS="${TAG_PATTERNS}type=pep440,pattern={{ major }}.{{ minor }},suffix=-${TAG},value=${VERSION}\n"
    TAG_PATTERNS="${TAG_PATTERNS}type=raw,value=${TAG}\n"
  fi
done

# Remove the trailing escaped newline from the pattern list.
TAG_PATTERNS="${TAG_PATTERNS%\\n}"

# Export the patterns using GitHub Actions' multiline environment syntax.
{
  echo "TAG_PATTERNS<<EOF"
  echo -e "${TAG_PATTERNS}"
  echo EOF
} >> "$GITHUB_ENV"
