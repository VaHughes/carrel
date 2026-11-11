#!/usr/bin/env bash
# Build in this checkout, resetting oversized Cargo output before the next build.
# Usage: ./scripts/build.sh [cargo build options, e.g. --release or --locked]
set -euo pipefail

# A convenience symlink must still build/clean the script's own checkout,
# not a different target/ beside the symlink. Plain readlink is portable to
# macOS too; resolve relative links against each link's physical directory.
build_script_path=${BASH_SOURCE[0]}
if [[ "$build_script_path" != /* ]]; then
  build_script_path="$PWD/$build_script_path"
fi
while [[ -L "$build_script_path" ]]; do
  build_script_dir=$(cd -P "$(dirname "$build_script_path")" && pwd)
  build_script_path=$(readlink "$build_script_path")
  if [[ "$build_script_path" != /* ]]; then
    build_script_path="$build_script_dir/$build_script_path"
  fi
done
cd -P "$(dirname "$build_script_path")/.."

# Keep cleanup confined to this workspace, even with a shared CARGO_TARGET_DIR.
for arg in "$@"; do
  case "$arg" in
    --target-dir|--target-dir=*|--manifest-path|--manifest-path=*|-m|-m?*|--config|--config=*)
      echo 'build.sh uses this checkout and its target/; use cargo build for path or config overrides.' >&2
      exit 2
      ;;
    -h|--help)
      echo 'Usage: ./scripts/build.sh [cargo build options]'
      echo 'Cleans target/ before building when it exceeds CARREL_BUILD_CACHE_MIB (default 4096).'
      echo 'Uses this checkout and its target/ regardless of CARGO_TARGET_DIR.'
      exit 0
      ;;
  esac
done

limit_mib=${CARREL_BUILD_CACHE_MIB:-4096}
if [[ ! "$limit_mib" =~ ^[1-9][0-9]{0,6}$ ]]; then
  echo 'CARREL_BUILD_CACHE_MIB must be a positive integer (at most 7 digits).' >&2
  exit 2
fi

# A symlink could point to another checkout's cache; never clean through it.
if [[ -L target ]]; then
  echo 'build.sh requires a local target/ directory; use cargo build for a symlinked target.' >&2
  exit 2
fi

# Cargo can keep intermediate artifacts in a separate build directory. A
# full clean removes that too, so pin it as well as the final target directory.
# --config is rejected above because it could override this environment value.
export CARGO_BUILD_BUILD_DIR="$PWD/target"

if [[ -d target ]]; then
  used_kib=$(du -sk target | awk '{print $1}')
  if (( used_kib > limit_mib * 1024 )); then
    printf 'Build cache is %s MiB (limit %s MiB); checking before cleanup.\n' \
      "$((used_kib / 1024))" "$limit_mib"
    # Cargo validates this marker before an explicit-target cleanup. Older
    # output may lack it; do not invent one or bypass Cargo's deletion guard.
    signature='Signature: 8a477f597d28d172789f06886806bc55'
    if [[ ! -f target/CACHEDIR.TAG || -L target/CACHEDIR.TAG ]] \
      || [[ $(head -c 43 target/CACHEDIR.TAG 2>/dev/null) != "$signature" ]]; then
      echo 'Skipping automatic cleanup: target/ has no valid Cargo cache marker.' >&2
    else
      cargo clean --target-dir "$PWD/target"
    fi
  fi
fi

exec cargo build --target-dir "$PWD/target" "$@"
