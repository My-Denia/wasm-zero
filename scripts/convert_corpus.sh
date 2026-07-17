#!/usr/bin/env bash
# Convert the full wg-2.0 test/core corpus (148 .wast files) to JSON + binary
# modules with the pinned wast2json. Output: build/wast-json/ (gitignored).
# simd files are prefixed simd_ to keep one flat output directory.
set -euo pipefail
cd "$(dirname "$0")/.."

W="third_party/wabt/wabt-1.0.41/bin/wast2json"
[ -x "$W" ] || W="$W.exe"
[ -x "$W" ] || { echo "wast2json missing; run scripts/fetch_wabt.sh" >&2; exit 1; }

# Recreate the output directory so stale artifacts from earlier runs can
# never ride along into the accounted corpus.
rm -rf build/wast-json
mkdir -p build/wast-json
ok=0; bad=0
for f in third_party/spec/test/core/*.wast; do
  b=$(basename "$f" .wast)
  if "$W" "$f" -o "build/wast-json/$b.json"; then ok=$((ok+1)); else bad=$((bad+1)); echo "FAILED: $f" >&2; fi
done
for f in third_party/spec/test/core/simd/*.wast; do
  b=$(basename "$f" .wast)
  if "$W" "$f" -o "build/wast-json/simd_$b.json"; then ok=$((ok+1)); else bad=$((bad+1)); echo "FAILED: $f" >&2; fi
done
echo "convert_corpus: ok=$ok bad=$bad"
[ "$bad" -eq 0 ] && [ "$ok" -eq 148 ]
