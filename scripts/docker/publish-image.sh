#!/usr/bin/env bash

set -euo pipefail

script_directory=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
build_id=$(jq -er '.build_id | select(test("^[a-zA-Z0-9]+$"))' <<< "$IMAGE_JSON")
digest=$(jq -er '.digest | select(test("^sha256:[0-9a-f]{64}$"))' <<< "$IMAGE_JSON")

# Verify that the Depot build ID resolves to the recorded index digest.
BUILD_ID="$build_id" "$script_directory/depot-login.sh"
saved_digest=$(docker buildx imagetools inspect \
  "registry.depot.dev/${DEPOT_PROJECT_ID}:$build_id" --format '{{.Manifest.Digest}}')
if [ "$saved_digest" != "$digest" ]; then
  echo "The saved Depot image does not match the build manifest" >&2
  exit 1
fi

# Keep annotation values containing spaces or shell-special characters
# together as individual command arguments.
annotations=()
while IFS= read -r annotation; do
  annotations+=(--annotation "$annotation")
done < <(jq -r '.annotations[]' <<< "$IMAGE_JSON")

# Only accept tag references for the destination repositories selected
# by this workflow.
for image in $IMAGES; do
  readarray -t image_tags < <(
    jq -r --arg prefix "$image:" '.tags[] | select(startswith($prefix))' <<< "$IMAGE_JSON"
  )
  if [ "${#image_tags[@]}" -eq 0 ]; then
    echo "Missing tags for $image" >&2
    exit 1
  fi

  # Copy the complete index instead of loading and re-pushing it through
  # the local Docker image store, which can drop platforms or change
  # the manifest format (astral-sh/uv#14165).
  "$script_directory/depot-push-with-retry.sh" --project "$DEPOT_PROJECT_ID" --tag "${image_tags[0]}" "$build_id"
  copied_digest=$(docker buildx imagetools inspect \
    "${image_tags[0]}" --format '{{.Manifest.Digest}}')
  if [ "$copied_digest" != "$digest" ]; then
    echo "The copied image digest does not match the build manifest" >&2
    exit 1
  fi

  tags=()
  for tag in "${image_tags[@]}"; do
    tags+=(-t "$tag")
  done
  # Apply index metadata during publication so the final digest covers
  # both the complete multi-platform image and its annotations.
  docker buildx imagetools create \
    "${annotations[@]}" \
    "${tags[@]}" \
    --metadata-file "$RUNNER_TEMP/docker-publish-metadata.json" \
    "$image@$digest"

  # Adding index annotations changes the digest. Read the new digest
  # from `imagetools` so the attestation cannot race a mutable tag.
  if [ "$image" == "$UV_GHCR_IMAGE" ]; then
    final_digest=$(jq -er '."containerimage.descriptor".digest' "$RUNNER_TEMP/docker-publish-metadata.json")
    echo "digest=$final_digest" >> "$GITHUB_OUTPUT"
  fi
done
