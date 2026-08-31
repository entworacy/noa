#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="$project_dir/iris-agent/build"
class_dir="$build_dir/classes"
dex_dir="$build_dir/dex"
r8_version="9.3.16"
r8_jar="$project_dir/.tools/r8/r8-$r8_version.jar"

mkdir -p "$class_dir" "$dex_dir" "$project_dir/assets" "$project_dir/.tools/r8"
find "$class_dir" "$dex_dir" -type f -delete

if [[ ! -f "$r8_jar" ]]; then
  curl -fsSL --retry 3 -o "$r8_jar" "https://dl.google.com/dl/android/maven2/com/android/tools/r8/$r8_version/r8-$r8_version.jar"
fi

javac --release 8 -encoding UTF-8 -d "$class_dir" $(find "$project_dir/iris-agent/java" -name '*.java' -print)
jar --create --file "$build_dir/classes.jar" -C "$class_dir" .
java -cp "$r8_jar" com.android.tools.r8.D8 --min-api 26 --output "$dex_dir" "$build_dir/classes.jar"
cp "$dex_dir/classes.dex" "$project_dir/assets/noa-iris-agent.dex"
