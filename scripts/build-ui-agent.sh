#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="$project_dir/ui-agent/build"
stub_dir="$build_dir/stubs"
class_dir="$build_dir/classes"
dex_dir="$build_dir/dex"
r8_version="9.3.16"
r8_dir="$project_dir/.tools/r8"
r8_jar="$r8_dir/r8-$r8_version.jar"

command -v javac >/dev/null
command -v java >/dev/null
command -v jar >/dev/null
command -v curl >/dev/null

mkdir -p "$stub_dir" "$class_dir" "$dex_dir" "$r8_dir"
find "$stub_dir" "$class_dir" "$dex_dir" -type f -delete

if [[ ! -f "$r8_jar" ]]; then
  curl -fsSL --retry 3 -o "$r8_jar" "https://dl.google.com/dl/android/maven2/com/android/tools/r8/$r8_version/r8-$r8_version.jar"
fi

javac --release 8 -encoding UTF-8 -d "$stub_dir" $(find "$project_dir/ui-agent/stubs" -name '*.java' -print)
javac --release 8 -encoding UTF-8 -cp "$stub_dir" -d "$class_dir" $(find "$project_dir/ui-agent/src" -name '*.java' -print)
jar --create --file "$build_dir/classes.jar" -C "$class_dir" .
java -cp "$r8_jar" com.android.tools.r8.D8 --min-api 26 --output "$dex_dir" "$build_dir/classes.jar"
jar --create --file "$project_dir/assets/noa-uiautomator.jar" -C "$dex_dir" classes.dex
