#!/usr/bin/env bash
# Fetch the official WebAssembly spec repo pinned at the wg-2.0 tag.
# Corpus + oracle sources live under third_party/spec (gitignored).
set -euo pipefail
cd "$(dirname "$0")/.."

SPEC_REPO="https://github.com/WebAssembly/spec"
SPEC_TAG="wg-2.0"
SPEC_SHA="fffc6e12fa454e475455a7b58d3b5dc343980c10"

if [ -d third_party/spec ]; then
  have=$(git -C third_party/spec rev-parse HEAD)
  if [ "$have" = "$SPEC_SHA" ]; then
    echo "spec already at $SPEC_SHA"
    exit 0
  fi
  echo "spec present at wrong SHA ($have), refusing to touch it" >&2
  exit 1
fi

mkdir -p third_party
git clone --depth 1 --branch "$SPEC_TAG" "$SPEC_REPO" third_party/spec
have=$(git -C third_party/spec rev-parse HEAD)
if [ "$have" != "$SPEC_SHA" ]; then
  echo "pinned SHA mismatch: expected $SPEC_SHA got $have" >&2
  exit 1
fi
echo "spec fetched at $SPEC_SHA"
