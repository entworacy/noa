#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="$project_dir/ui-agent/build"
class_dir="$build_dir/classes"
dex_dir="$build_dir/dex"
r8_version="9.3.16"
r8_dir="$project_dir/.tools/r8"
r8_jar="$r8_dir/r8-$r8_version.jar"
android_api_dir="$project_dir/.tools/android-api-26"
android_jar="$android_api_dir/android.jar"
uiautomator_jar="$android_api_dir/uiautomator.jar"
android_jar_sha256="cdc1846376a14b0370cc63454a129606b4a52cc50ada75ef0d4cf956b1ad2daa"
uiautomator_jar_sha256="7450969cd771107c03b9a7fa0d143ac2fba021e0c709ae6fc7935d107f0a9eb5"
android_jar_url="https://android.googlesource.com/platform/prebuilts/fullsdk/platforms/android-26/+/refs/heads/main/android.jar?format=TEXT"
uiautomator_jar_url="https://android.googlesource.com/platform/prebuilts/fullsdk/platforms/android-26/+/refs/heads/main/uiautomator.jar?format=TEXT"

command -v javac >/dev/null
command -v javap >/dev/null
command -v java >/dev/null
command -v jar >/dev/null
command -v curl >/dev/null
command -v base64 >/dev/null
command -v sha256sum >/dev/null

mkdir -p "$class_dir" "$dex_dir" "$r8_dir" "$android_api_dir"
find "$class_dir" "$dex_dir" -type f -delete

ensure_api_jar() {
  local path="$1"
  local url="$2"
  local expected_sha256="$3"
  if [[ -f "$path" ]] && [[ "$(sha256sum "$path" | awk '{print $1}')" == "$expected_sha256" ]]; then
    return
  fi
  local staging="$path.install"
  curl -fsSL --retry 3 "$url" | base64 -d >"$staging"
  local actual_sha256
  actual_sha256="$(sha256sum "$staging" | awk '{print $1}')"
  if [[ "$actual_sha256" != "$expected_sha256" ]]; then
    rm -f "$staging"
    printf 'Android API jar 체크섬 불일치: %s\n' "$path" >&2
    exit 1
  fi
  mv "$staging" "$path"
}

ensure_api_jar "$android_jar" "$android_jar_url" "$android_jar_sha256"
ensure_api_jar "$uiautomator_jar" "$uiautomator_jar_url" "$uiautomator_jar_sha256"

if [[ ! -f "$r8_jar" ]]; then
  curl -fsSL --retry 3 -o "$r8_jar" "https://dl.google.com/dl/android/maven2/com/android/tools/r8/$r8_version/r8-$r8_version.jar"
fi

javap -classpath "$uiautomator_jar:$android_jar" \
  com.android.uiautomator.core.Configurator \
  com.android.uiautomator.core.UiDevice \
  com.android.uiautomator.core.UiObject \
  com.android.uiautomator.core.UiScrollable \
  com.android.uiautomator.core.UiSelector \
  com.android.uiautomator.testrunner.UiAutomatorTestCase \
  android.content.ClipData \
  android.content.ClipData\$Item \
  android.app.UiAutomation \
  android.view.accessibility.AccessibilityNodeInfo \
  android.view.accessibility.AccessibilityWindowInfo >"$build_dir/api-stubs.txt"

required_api_signatures=(
  'public static com.android.uiautomator.core.Configurator getInstance()'
  'public com.android.uiautomator.core.Configurator setWaitForIdleTimeout(long)'
  'public com.android.uiautomator.core.Configurator setWaitForSelectorTimeout(long)'
  'public com.android.uiautomator.core.Configurator setActionAcknowledgmentTimeout(long)'
  'public com.android.uiautomator.core.Configurator setScrollAcknowledgmentTimeout(long)'
  'public void setCompressedLayoutHeirarchy(boolean)'
  'public void dumpWindowHierarchy(java.lang.String)'
  'public boolean click(int, int)'
  'public boolean swipe(int, int, int, int, int)'
  'public void waitForIdle(long)'
  'public com.android.uiautomator.core.UiObject(com.android.uiautomator.core.UiSelector)'
  'public boolean exists()'
  'public boolean click()'
  'public android.graphics.Rect getBounds()'
  'public java.lang.String getText()'
  'public java.lang.String getContentDescription()'
  'public com.android.uiautomator.core.UiScrollable(com.android.uiautomator.core.UiSelector)'
  'public com.android.uiautomator.core.UiScrollable setAsVerticalList()'
  'public boolean scrollToBeginning(int, int)'
  'public boolean scrollForward(int)'
  'public com.android.uiautomator.core.UiSelector description(java.lang.String)'
  'public com.android.uiautomator.core.UiSelector descriptionMatches(java.lang.String)'
  'public com.android.uiautomator.core.UiSelector text(java.lang.String)'
  'public com.android.uiautomator.core.UiSelector textMatches(java.lang.String)'
  'public com.android.uiautomator.core.UiSelector resourceId(java.lang.String)'
  'public com.android.uiautomator.core.UiSelector resourceIdMatches(java.lang.String)'
  'public com.android.uiautomator.core.UiSelector className(java.lang.String)'
  'public com.android.uiautomator.core.UiSelector scrollable(boolean)'
  'public com.android.uiautomator.core.UiSelector instance(int)'
  'public com.android.uiautomator.core.UiDevice getUiDevice()'
  'public int getItemCount()'
  'public android.content.ClipData$Item getItemAt(int)'
  'public java.lang.CharSequence getText()'
  'public java.util.List<android.view.accessibility.AccessibilityNodeInfo> findAccessibilityNodeInfosByViewId(java.lang.String)'
  'public java.lang.CharSequence getContentDescription()'
  'public int getChildCount()'
  'public android.view.accessibility.AccessibilityNodeInfo getChild(int)'
  'public android.view.accessibility.AccessibilityNodeInfo getParent()'
  'public boolean isClickable()'
  'public boolean performAction(int)'
  'public void getBoundsInScreen(android.graphics.Rect)'
  'public void recycle()'
  'public java.util.List<android.view.accessibility.AccessibilityWindowInfo> getWindows()'
  'public android.view.accessibility.AccessibilityNodeInfo getRootInActiveWindow()'
  'public boolean isActive()'
  'public boolean isFocused()'
  'public android.view.accessibility.AccessibilityNodeInfo getRoot()'
)
for signature in "${required_api_signatures[@]}"; do
  if ! grep -Fq "$signature" "$build_dir/api-stubs.txt"; then
    printf 'UIAutomator API 스텁 시그니처 누락: %s\n' "$signature" >&2
    exit 1
  fi
done

javac -source 8 -target 8 -encoding UTF-8 \
  -bootclasspath "$android_jar" \
  -classpath "$uiautomator_jar" \
  -d "$class_dir" \
  $(find "$project_dir/ui-agent/src" -name '*.java' -print)
jar --create --date=1980-01-01T00:00:02Z --file "$build_dir/classes.jar" -C "$class_dir" .
java -cp "$r8_jar" com.android.tools.r8.D8 --min-api 26 --output "$dex_dir" "$build_dir/classes.jar"
jar --create --date=1980-01-01T00:00:02Z --file "$project_dir/assets/noa-uiautomator.jar" -C "$dex_dir" classes.dex
