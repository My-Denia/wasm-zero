#!/usr/bin/env bash
# Fetch the official WABT release (wast2json is the pinned text-format frontend).
# SHA256-verified; unpacked under third_party/wabt (gitignored).
set -euo pipefail
cd "$(dirname "$0")/.."

WABT_VERSION="1.0.41"
case "$(uname -s)" in
  Linux)  PLAT="linux-x64";   SHA="83f8122e924745fcd70636e3594bc01c4c47f2d4c8f3c63b5d70d3f83a482677" ;;
  MINGW*|MSYS*|CYGWIN*) PLAT="windows-x64"; SHA="37285ec7244384ffd382841f93fd23335aae846c92016a132d765c60f27a2f31" ;;
  *) echo "unsupported platform $(uname -s)" >&2; exit 1 ;;
esac

BIN="third_party/wabt/wabt-$WABT_VERSION/bin"
if [ -x "$BIN/wast2json" ] || [ -x "$BIN/wast2json.exe" ]; then
  echo "wabt $WABT_VERSION already present"
  exit 0
fi

mkdir -p third_party/wabt
TARBALL="third_party/wabt/wabt.tar.gz"
curl -sL -o "$TARBALL" \
  "https://github.com/WebAssembly/wabt/releases/download/$WABT_VERSION/wabt-$WABT_VERSION-$PLAT.tar.gz"
echo "$SHA  $TARBALL" | sha256sum -c -
tar xzf "$TARBALL" -C third_party/wabt
rm -f "$TARBALL"
echo "wabt $WABT_VERSION ($PLAT) fetched and verified"
