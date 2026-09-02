#include "lsplant_shorty.hpp"

#include <string>
#include <string_view>

namespace {
thread_local jmethodID fallback_method = nullptr;
thread_local std::string fallback_value;

bool jni_ok(JNIEnv *env) {
    if (!env->ExceptionCheck()) return true;
    env->ExceptionClear();
    return false;
}

char reflected_type_shorty(JNIEnv *env, jobject type, jmethodID get_name) {
    auto name = static_cast<jstring>(env->CallObjectMethod(type, get_name));
    if (name == nullptr || !jni_ok(env)) return '\0';
    const char *utf = env->GetStringUTFChars(name, nullptr);
    if (utf == nullptr || !jni_ok(env)) return '\0';
    std::string_view value(utf);
    char shorty = 'L';
    if (value == "void") shorty = 'V';
    else if (value == "boolean") shorty = 'Z';
    else if (value == "byte") shorty = 'B';
    else if (value == "char") shorty = 'C';
    else if (value == "short") shorty = 'S';
    else if (value == "int") shorty = 'I';
    else if (value == "long") shorty = 'J';
    else if (value == "float") shorty = 'F';
    else if (value == "double") shorty = 'D';
    env->ReleaseStringUTFChars(name, utf);
    return shorty;
}
}

namespace noa::lsplant {
const char *reflection_get_method_shorty(JNIEnv *, jmethodID method) {
    return method == fallback_method && !fallback_value.empty() ? fallback_value.c_str() : nullptr;
}

bool prepare_reflection_shorty(JNIEnv *env, jobject target) {
    clear_reflection_shorty();
    if (env->PushLocalFrame(32) != JNI_OK) {
        jni_ok(env);
        return false;
    }
    const bool prepared = [&]() {
        auto executable = env->FindClass("java/lang/reflect/Executable");
        auto method_class = env->FindClass("java/lang/reflect/Method");
        auto class_class = env->FindClass("java/lang/Class");
        if (executable == nullptr || method_class == nullptr || class_class == nullptr ||
            !jni_ok(env)) {
            return false;
        }
        auto get_parameters = env->GetMethodID(
            executable, "getParameterTypes", "()[Ljava/lang/Class;");
        auto get_return = env->GetMethodID(method_class, "getReturnType", "()Ljava/lang/Class;");
        auto get_name = env->GetMethodID(class_class, "getName", "()Ljava/lang/String;");
        if (get_parameters == nullptr || get_return == nullptr || get_name == nullptr ||
            !jni_ok(env)) {
            return false;
        }

        char return_shorty = 'V';
        if (env->IsInstanceOf(target, method_class)) {
            auto return_type = env->CallObjectMethod(target, get_return);
            if (return_type == nullptr || !jni_ok(env)) return false;
            return_shorty = reflected_type_shorty(env, return_type, get_name);
            if (return_shorty == '\0') return false;
        }
        auto parameters = static_cast<jobjectArray>(env->CallObjectMethod(target, get_parameters));
        if (parameters == nullptr || !jni_ok(env)) return false;
        fallback_value.push_back(return_shorty);
        const auto count = env->GetArrayLength(parameters);
        for (jsize index = 0; index < count; ++index) {
            auto type = env->GetObjectArrayElement(parameters, index);
            if (type == nullptr || !jni_ok(env)) return false;
            const char shorty = reflected_type_shorty(env, type, get_name);
            if (shorty == '\0') return false;
            fallback_value.push_back(shorty);
        }
        fallback_method = env->FromReflectedMethod(target);
        return fallback_method != nullptr && jni_ok(env);
    }();
    env->PopLocalFrame(nullptr);
    if (!prepared) clear_reflection_shorty();
    return prepared;
}

void clear_reflection_shorty() {
    fallback_method = nullptr;
    fallback_value.clear();
}
}
