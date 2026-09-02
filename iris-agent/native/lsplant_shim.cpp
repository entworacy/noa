#include "lsplant_api.hpp"
#include "lsplant_shorty.hpp"
#include "xdl.h"
#include <android/dlext.h>
#include <dlfcn.h>
#include <frida-gum.h>
#include <string>
#include <vector>

namespace {
GumInterceptor *interceptor = nullptr;
GumModule *art = nullptr;
void *art_xdl = nullptr;
std::string last_error;
std::vector<std::string> unresolved_symbols;
std::vector<std::string> unresolved_prefixes;
size_t xdl_resolutions = 0;
size_t gum_resolutions = 0;
size_t inline_hook_failures = 0;
bool shorty_fallback_enabled = false;

using Init = bool (*)(JNIEnv *, const lsplant::InitInfo &);
using Hook = jobject (*)(JNIEnv *, jobject, jobject, jobject);
using Deoptimize = bool (*)(JNIEnv *, jobject);

void remember_unresolved(std::vector<std::string> &values, std::string_view value) {
    if (values.size() >= 12) return;
    for (const auto &existing : values) {
        if (existing == value) return;
    }
    values.emplace_back(value);
}

void append_values(std::string &message, std::string_view label,
                   const std::vector<std::string> &values) {
    if (values.empty()) return;
    message.append("; ").append(label).append("=[");
    for (size_t index = 0; index < values.size(); ++index) {
        if (index != 0) message.append(", ");
        message.append(values[index]);
    }
    message.push_back(']');
}

void *inline_hook(void *target, void *replacement) {
    gpointer original = nullptr;
    gum_interceptor_begin_transaction(interceptor);
    auto result = gum_interceptor_replace(interceptor, target, replacement, nullptr, &original);
    gum_interceptor_end_transaction(interceptor);
    gum_interceptor_flush(interceptor);
    if (result == GUM_REPLACE_OK) return original;
    ++inline_hook_failures;
    return nullptr;
}

bool inline_unhook(void *target) {
    gum_interceptor_begin_transaction(interceptor);
    gum_interceptor_revert(interceptor, target);
    gum_interceptor_end_transaction(interceptor);
    gum_interceptor_flush(interceptor);
    return true;
}

void *resolve_symbol(std::string_view name) {
    std::string owned(name);
    if (art_xdl != nullptr) {
        if (auto *symbol = xdl_sym(art_xdl, owned.c_str(), nullptr); symbol != nullptr) {
            ++xdl_resolutions;
            return symbol;
        }
        if (auto *symbol = xdl_dsym(art_xdl, owned.c_str(), nullptr); symbol != nullptr) {
            ++xdl_resolutions;
            return symbol;
        }
    }
    auto *symbol = art == nullptr
        ? nullptr
        : reinterpret_cast<void *>(gum_module_find_symbol_by_name(art, owned.c_str()));
    if (symbol != nullptr) {
        ++gum_resolutions;
        return symbol;
    }
    if (name == "_ZN3artL15GetMethodShortyEP7_JNIEnvP10_jmethodID" ||
        name == "_ZN3art15GetMethodShortyEP7_JNIEnvP10_jmethodID") {
        shorty_fallback_enabled = true;
        return reinterpret_cast<void *>(noa::lsplant::reflection_get_method_shorty);
    }
    remember_unresolved(unresolved_symbols, name);
    return nullptr;
}

struct PrefixSearch {
    std::string_view prefix;
    void *result;
};

gboolean find_prefix(const GumSymbolDetails *details, gpointer data) {
    auto *search = static_cast<PrefixSearch *>(data);
    std::string_view name(details->name);
    if (!name.starts_with(search->prefix)) return true;
    search->result = reinterpret_cast<void *>(details->address);
    return false;
}

void *resolve_prefix(std::string_view prefix) {
    PrefixSearch search{prefix, nullptr};
    if (art != nullptr) gum_module_enumerate_symbols(art, find_prefix, &search);
    if (search.result != nullptr) {
        ++gum_resolutions;
    } else {
        remember_unresolved(unresolved_prefixes, prefix);
    }
    return search.result;
}
}

extern "C" void *noa_dlopen_fd(int fd, int flags) {
    android_dlextinfo info{};
    info.flags = ANDROID_DLEXT_USE_LIBRARY_FD;
    info.library_fd = fd;
    return android_dlopen_ext("noa-lsplant.so", flags, &info);
}

extern "C" bool noa_lsplant_init(JNIEnv *env, void *handle) {
    last_error.clear();
    unresolved_symbols.clear();
    unresolved_prefixes.clear();
    xdl_resolutions = 0;
    gum_resolutions = 0;
    inline_hook_failures = 0;
    shorty_fallback_enabled = false;
    auto init = reinterpret_cast<Init>(dlsym(handle, "_ZN7lsplant2v24InitEP7_JNIEnvRKNS0_8InitInfoE"));
    if (init == nullptr) {
        last_error = "LSPlant Init export is missing";
        return false;
    }
    gum_init_embedded();
    interceptor = gum_interceptor_obtain();
    art = gum_process_find_module_by_name("libart.so");
    art_xdl = xdl_open("libart.so", XDL_DEFAULT);
    if (interceptor == nullptr) {
        last_error = "Frida Gum interceptor is unavailable";
        return false;
    }
    if (art == nullptr) {
        last_error = "Frida Gum could not find libart.so";
        return false;
    }
    lsplant::InitInfo info{
        .inline_hooker = inline_hook,
        .inline_unhooker = inline_unhook,
        .art_symbol_resolver = resolve_symbol,
        .art_symbol_prefix_resolver = resolve_prefix,
    };
    if (init(env, info)) return true;

    last_error = "LSPlant Init returned false; symbol resolver=";
    last_error.append(art_xdl == nullptr ? "Frida-only" : "xDL+Frida");
    last_error.append("; resolved(xDL=").append(std::to_string(xdl_resolutions));
    last_error.append(", Frida=").append(std::to_string(gum_resolutions)).append(")");
    append_values(last_error, "unresolved symbols", unresolved_symbols);
    append_values(last_error, "unresolved prefixes", unresolved_prefixes);
    if (inline_hook_failures != 0) {
        last_error.append("; inline hook failures=").append(std::to_string(inline_hook_failures));
    }
    return false;
}

extern "C" const char *noa_lsplant_last_error() {
    return last_error.empty() ? "unknown LSPlant initialization error" : last_error.c_str();
}

extern "C" bool noa_lsplant_uses_shorty_fallback() {
    return shorty_fallback_enabled;
}

extern "C" jobject noa_lsplant_hook(
    JNIEnv *env,
    void *handle,
    jobject target,
    jobject hooker,
    jobject callback
) {
    auto hook = reinterpret_cast<Hook>(dlsym(handle, "_ZN7lsplant2v24HookEP7_JNIEnvP8_jobjectS4_S4_"));
    if (hook == nullptr) return nullptr;
    if (shorty_fallback_enabled && !noa::lsplant::prepare_reflection_shorty(env, target)) {
        return nullptr;
    }
    auto result = hook(env, target, hooker, callback);
    noa::lsplant::clear_reflection_shorty();
    return result;
}

extern "C" bool noa_lsplant_deoptimize(JNIEnv *env, void *handle, jobject target) {
    auto deoptimize = reinterpret_cast<Deoptimize>(dlsym(handle, "_ZN7lsplant2v210DeoptimizeEP7_JNIEnvP8_jobject"));
    return deoptimize != nullptr && deoptimize(env, target);
}
