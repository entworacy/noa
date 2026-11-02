#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="$(mktemp -d)"
trap 'rm -rf "$build_dir"' EXIT
source_dir="$project_dir/kakao-agent/java/dev/noa/kakao"
fixture_dir="$project_dir/kakao-agent/tests/java/dev/noa/kakao"
javac --release 8 -d "$build_dir" \
  "$source_dir/LocoSender.java" "$source_dir/LocoPacketReader.java" \
  "$source_dir/Hooker.java" "$source_dir/KakaoReflection.java" \
  "$fixture_dir/KakaoSignatureResolver.java" "$fixture_dir/LocoSenderTest.java"
java -cp "$build_dir" dev.noa.kakao.LocoSenderTest
