#!/usr/bin/env bash
# Build the Pass 4 mobile creator Rust FFI library for Android ABIs.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

PROFILE="debug"
ABIS=()

usage() {
  cat <<'EOF'
Usage: build-mobile-ffi.sh [--abi arm64-v8a] [--abi x86_64] [--profile debug|release]

Builds crates/gbn-bridge-mobile-ffi for Android targets and copies shared libraries to:
  mobile/android/app/src/main/jniLibs/<abi>/libgbn_bridge_mobile_ffi.so

The script requires WSL2 Ubuntu, rustup, and Android NDK configuration through either
ANDROID_NDK_HOME or ANDROID_HOME/ndk/<version>.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --abi)
      ABIS+=("$2")
      shift 2
      ;;
    --profile)
      PROFILE="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

[[ "${#ABIS[@]}" -gt 0 ]] || ABIS=(arm64-v8a x86_64)

uname -a | grep -i microsoft >/dev/null || {
  echo "Pass 4 mobile FFI build requires WSL2 Ubuntu" >&2
  exit 1
}

command -v cargo >/dev/null || { echo "cargo is required" >&2; exit 1; }
command -v rustup >/dev/null || { echo "rustup is required" >&2; exit 1; }

if [[ "$PROFILE" != "debug" && "$PROFILE" != "release" ]]; then
  echo "--profile must be debug or release" >&2
  exit 2
fi

if [[ -n "${ANDROID_NDK_HOME:-}" ]]; then
  NDK_HOME="$ANDROID_NDK_HOME"
elif [[ -n "${ANDROID_HOME:-}" && -d "$ANDROID_HOME/ndk" ]]; then
  NDK_HOME="$(find "$ANDROID_HOME/ndk" -mindepth 1 -maxdepth 1 -type d | sort -V | tail -1)"
else
  echo "Android NDK not found. Set ANDROID_NDK_HOME or ANDROID_HOME with an ndk/ install." >&2
  exit 1
fi

[[ -d "$NDK_HOME/toolchains/llvm/prebuilt" ]] || {
  echo "Invalid Android NDK path: $NDK_HOME" >&2
  exit 1
}

HOST_TAG=""
for candidate in linux-x86_64 linux; do
  if [[ -d "$NDK_HOME/toolchains/llvm/prebuilt/$candidate/bin" ]]; then
    HOST_TAG="$candidate"
    break
  fi
done
[[ -n "$HOST_TAG" ]] || {
  echo "Could not find Android NDK LLVM prebuilt bin directory under $NDK_HOME" >&2
  exit 1
}

target_for_abi() {
  case "$1" in
    arm64-v8a) echo "aarch64-linux-android" ;;
    x86_64) echo "x86_64-linux-android" ;;
    *) echo "Unsupported ABI: $1" >&2; return 2 ;;
  esac
}

linker_for_target() {
  local target="$1" api="${ANDROID_API_LEVEL:-26}"
  case "$target" in
    aarch64-linux-android) echo "$NDK_HOME/toolchains/llvm/prebuilt/$HOST_TAG/bin/aarch64-linux-android${api}-clang" ;;
    x86_64-linux-android) echo "$NDK_HOME/toolchains/llvm/prebuilt/$HOST_TAG/bin/x86_64-linux-android${api}-clang" ;;
    *) return 2 ;;
  esac
}

metadata_dir="$ROOT_DIR/mobile/android/app/src/main/jniLibs"
mkdir -p "$metadata_dir"

cd "$ROOT_DIR"

for abi in "${ABIS[@]}"; do
  target="$(target_for_abi "$abi")"
  linker="$(linker_for_target "$target")"
  [[ -x "$linker" ]] || {
    echo "Android linker not found or not executable: $linker" >&2
    exit 1
  }
  rustup target add "$target" >/dev/null
  env_var="CARGO_TARGET_$(echo "$target" | tr '[:lower:]-' '[:upper:]_')_LINKER"
  export "$env_var=$linker"
  cargo_args=(-p gbn-bridge-mobile-ffi --target "$target")
  if [[ "$PROFILE" == "release" ]]; then
    cargo_args+=(--release)
  fi
  cargo build "${cargo_args[@]}"
  out_dir="$metadata_dir/$abi"
  mkdir -p "$out_dir"
  profile_dir="$PROFILE"
  cp "target/$target/$profile_dir/libgbn_bridge_mobile_ffi.so" "$out_dir/libgbn_bridge_mobile_ffi.so"
done

metadata="$metadata_dir/mobile-ffi-build-metadata.json"
{
  echo "{"
  echo "  \"created_at_utc\": \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\","
  echo "  \"git_sha\": \"$(git rev-parse HEAD)\","
  echo "  \"rustc\": \"$(rustc --version)\","
  echo "  \"profile\": \"$PROFILE\","
  echo "  \"abis\": [$(printf '"%s",' "${ABIS[@]}" | sed 's/,$//')]"
  echo "}"
} >"$metadata"

echo "Mobile FFI Android artifacts written under $metadata_dir"
