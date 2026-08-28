#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

if ! command -v cargo-ndk >/dev/null 2>&1; then
  echo "cargo-ndk가 필요합니다: cargo install cargo-ndk" >&2
  exit 1
fi
if [[ -z "${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}" ]]; then
  echo "ANDROID_NDK_HOME 또는 ANDROID_NDK_ROOT를 지정하세요." >&2
  exit 1
fi
for command in curl tar unzip javac java jar; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "$command 명령이 필요합니다." >&2
    exit 1
  fi
done

echo "[noa] building embedded UiAutomator agent"
"$project_dir/scripts/build-ui-agent.sh"
echo "[noa] building embedded KakaoTalk JNI adapter"
"$project_dir/scripts/build-kakao-agent.sh"
echo "[noa] building embedded Iris JNI adapter"
"$project_dir/scripts/build-iris-agent.sh"

mkdir -p dist

abis=("arm64-v8a" "armeabi-v7a" "x86" "x86_64")
targets=("aarch64-linux-android" "armv7-linux-androideabi" "i686-linux-android" "x86_64-linux-android")
frida_arches=("arm64" "arm" "x86" "x86_64")
compiler_arches=("aarch64" "arm" "i686" "x86_64")
clang_targets=("aarch64-linux-android" "armv7a-linux-androideabi" "i686-linux-android" "x86_64-linux-android")
library_targets=("aarch64-linux-android" "arm-linux-androideabi" "i686-linux-android" "x86_64-linux-android")
frida_version="16.7.19"
lsplant_version="6.4"
xdl_version="2.4.0"
ndk_home="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
llvm_readelf="$ndk_home/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf"

assert_agent_runtime_resolved() {
  local library="$1"
  if "$llvm_readelf" --dyn-syms "$library" \
    | awk '$7 == "UND" && $8 == "__clear_cache" { found = 1 } END { exit !found }'; then
    echo "$(basename "$library")에 미해결 __clear_cache 심볼이 남아 있습니다." >&2
    exit 1
  fi
  if "$llvm_readelf" --dyn-syms "$library" \
    | awk '
        $5 == "GLOBAL" && $7 == "UND" {
          sub(/@.*/, "", $8)
          if ($8 ~ /^_Z/ || $8 ~ /^__cxa_/ || $8 == "__gxx_personality_v0") {
            if ($8 != "__cxa_atexit" && $8 != "__cxa_finalize") found = 1
          }
        }
        END { exit !found }
      '; then
    echo "$(basename "$library")에 미해결 C++ 런타임 심볼이 남아 있습니다." >&2
    exit 1
  fi
}

assert_frida_selinux_patch_linked() {
  local binary="$1"
  if ! "$llvm_readelf" --dyn-syms --wide "$binary" \
    | awk '$7 != "UND" && $8 == "frida_selinux_patch_policy" { found = 1 } END { exit !found }'; then
    echo "$(basename "$binary")에 Frida Android SELinux 정책 초기화 함수가 링크되지 않았습니다." >&2
    exit 1
  fi
}

prepare_frida_core() {
  local architecture="$1"
  local root="$project_dir/.tools/frida-core-devkits/$architecture"
  local archive="$project_dir/.tools/frida-core-devkits/frida-core-devkit-$frida_version-android-$architecture.tar.xz"
  if [[ ! -f "$root/libfrida-core.a" ]]; then
    command -v curl >/dev/null 2>&1 || {
      echo "curl이 필요합니다." >&2
      exit 1
    }
    command -v tar >/dev/null 2>&1 || {
      echo "tar가 필요합니다." >&2
      exit 1
    }
    mkdir -p "$root"
    if [[ ! -f "$archive" ]]; then
      curl -fL --retry 3 -o "$archive" "https://github.com/frida/frida/releases/download/$frida_version/frida-core-devkit-$frida_version-android-$architecture.tar.xz"
    fi
    tar -xJf "$archive" -C "$root"
  fi
  printf '%s\n' "$root"
}

prepare_frida_gum() {
  local architecture="$1"
  local root="$project_dir/.tools/frida-gum-devkits/$architecture"
  local archive="$project_dir/.tools/frida-gum-devkits/frida-gum-devkit-$frida_version-android-$architecture.tar.xz"
  if [[ ! -f "$root/libfrida-gum.a" ]]; then
    mkdir -p "$root"
    if [[ ! -f "$archive" ]]; then
      curl -fL --retry 3 -o "$archive" "https://github.com/frida/frida/releases/download/$frida_version/frida-gum-devkit-$frida_version-android-$architecture.tar.xz"
    fi
    tar -xJf "$archive" -C "$root"
  fi
  printf '%s\n' "$root"
}

prepare_lsplant() {
  local abi="$1"
  local root="$project_dir/.tools/lsplant-aar"
  local archive="$root/lsplant-standalone-$lsplant_version.aar"
  local library="$root/$abi/liblsplant.so"
  if [[ ! -f "$archive" ]]; then
    mkdir -p "$root"
    curl -fL --retry 3 -o "$archive" "https://repo1.maven.org/maven2/org/lsposed/lsplant/lsplant-standalone/$lsplant_version/lsplant-standalone-$lsplant_version.aar"
  fi
  if [[ ! -f "$library" ]]; then
    mkdir -p "$root/$abi"
    unzip -p "$archive" "prefab/modules/lsplant/libs/android.$abi/liblsplant.so" > "$library.tmp"
    mv "$library.tmp" "$library"
  fi
  printf '%s\n' "$library"
}

prepare_xdl() {
  local root="$project_dir/.tools/xdl-$xdl_version"
  local archive="$project_dir/.tools/xdl-$xdl_version.tar.gz"
  if [[ ! -f "$root/xdl/src/main/cpp/include/xdl.h" ]]; then
    mkdir -p "$root"
    if [[ ! -f "$archive" ]]; then
      curl -fL --retry 3 -o "$archive" \
        "https://github.com/hexhacking/xDL/archive/refs/tags/v$xdl_version.tar.gz"
    fi
    tar -xzf "$archive" --strip-components=1 -C "$root"
  fi
  printf '%s\n' "$root"
}

build_lsplant_shim() {
  local abi="$1"
  local clang_target="$2"
  local gum="$3"
  local xdl="$4"
  local root="$project_dir/iris-agent/build/native/$abi"
  local c_compiler="$ndk_home/toolchains/llvm/prebuilt/linux-x86_64/bin/${clang_target}26-clang"
  local cxx_compiler="$ndk_home/toolchains/llvm/prebuilt/linux-x86_64/bin/${clang_target}26-clang++"
  local archiver="$ndk_home/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-ar"
  local xdl_source="$xdl/xdl/src/main/cpp"
  mkdir -p "$root"
  "$cxx_compiler" -std=c++20 -fPIC -Oz -ffunction-sections -fdata-sections -fvisibility=hidden \
    -I"$project_dir/iris-agent/native" -I"$gum" -I"$xdl_source/include" \
    -c "$project_dir/iris-agent/native/lsplant_shim.cpp" -o "$root/lsplant_shim.o"
  local objects=("$root/lsplant_shim.o")
  local source
  for source in "$xdl_source"/*.c; do
    local object="$root/$(basename "${source%.c}").o"
    "$c_compiler" -std=c17 -fPIC -Oz -ffunction-sections -fdata-sections -fvisibility=hidden \
      -I"$xdl_source/include" -I"$xdl_source" -c "$source" -o "$object"
    objects+=("$object")
  done
  "$archiver" rcs "$root/libnoa_lsplant_shim.a" "${objects[@]}"
  printf '%s\n' "$root/libnoa_lsplant_shim.a"
}

xdl_root="$(prepare_xdl)"

for index in "${!abis[@]}"; do
  abi="${abis[$index]}"
  target="${targets[$index]}"
  frida_arch="${frida_arches[$index]}"
  compiler_arch="${compiler_arches[$index]}"
  clang_target="${clang_targets[$index]}"
  library_target="${library_targets[$index]}"
  frida_core="$(prepare_frida_core "$frida_arch")"
  frida_gum="$(prepare_frida_gum "$frida_arch")"
  lsplant="$(prepare_lsplant "$abi")"
  lsplant_shim="$(build_lsplant_shim "$abi" "$clang_target" "$frida_gum" "$xdl_root")"
  cxx_runtime_dir="$ndk_home/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/$library_target"
  [[ -f "$cxx_runtime_dir/libc++_static.a" && -f "$cxx_runtime_dir/libc++abi.a" ]] || {
    echo "$abi Android C++ runtime을 찾지 못했습니다." >&2
    exit 1
  }
  compiler_runtime="$(find "$ndk_home/toolchains/llvm/prebuilt" -name "libclang_rt.builtins-$compiler_arch-android.a" -print -quit)"
  [[ -n "$compiler_runtime" ]] || {
    echo "$abi Android compiler runtime을 찾지 못했습니다." >&2
    exit 1
  }
  echo "[noa] building $abi ($target)"
  NOA_FRIDA_GUM_DEVKIT="$frida_gum" NOA_LSPLANT_SHIM="$lsplant_shim" NOA_LSPLANT_BLOB="$lsplant" NOA_CXX_RUNTIME_DIR="$cxx_runtime_dir" NOA_COMPILER_RUNTIME="$compiler_runtime" \
    cargo ndk -t "$abi" -P 26 build --release --manifest-path "$project_dir/kakao-agent/Cargo.toml" --locked
  kakao_agent="$project_dir/kakao-agent/target/$target/release/libnoa_kakao_agent.so"
  assert_agent_runtime_resolved "$kakao_agent"
  NOA_FRIDA_GUM_DEVKIT="$frida_gum" NOA_LSPLANT_SHIM="$lsplant_shim" NOA_LSPLANT_BLOB="$lsplant" NOA_CXX_RUNTIME_DIR="$cxx_runtime_dir" NOA_COMPILER_RUNTIME="$compiler_runtime" \
    cargo ndk -t "$abi" -P 26 build --release --manifest-path "$project_dir/iris-agent/Cargo.toml" --locked
  iris_agent="$project_dir/iris-agent/target/$target/release/libnoa_iris_agent.so"
  assert_agent_runtime_resolved "$iris_agent"
  NOA_FRIDA_CORE_DEVKIT="$frida_core" NOA_KAKAO_AGENT_BLOB="$kakao_agent" NOA_IRIS_AGENT_BLOB="$iris_agent" RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C link-arg=$compiler_runtime" cargo ndk -t "$abi" -P 26 build --release --locked
  noa_binary="$project_dir/target/$target/release/noa"
  assert_frida_selinux_patch_linked "$noa_binary"
  cp "$noa_binary" "dist/noa-$abi"
done

echo "[noa] Android binaries are in $project_dir/dist"
