#pragma once

#include <jni.h>

namespace noa::lsplant {
const char *reflection_get_method_shorty(JNIEnv *env, jmethodID method);
bool prepare_reflection_shorty(JNIEnv *env, jobject target);
void clear_reflection_shorty();
}
