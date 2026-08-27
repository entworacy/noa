#include "lsplant_api.hpp"
#include <dlfcn.h>
#include <frida-gum.h>
#include <string>

namespace {
GumInterceptor *interceptor = nullptr;
GumModule *art = nullptr;

using Init = bool (*)(JNIEnv *, const lsplant::InitInfo &);
using Hook = jobject (*)(JNIEnv *, jobject, jobject, jobject);
using Deoptimize = bool (*)(JNIEnv *, jobject);

void *inline_hook(void *target, void *replacement) {
    gpointer original = nullptr;
    gum_interceptor_begin_transaction(interceptor);
    auto result = gum_interceptor_replace(interceptor, target, replacement, nullptr, &original);
    gum_interceptor_end_transaction(interceptor);
    gum_interceptor_flush(interceptor);
    return result == GUM_REPLACE_OK ? original : nullptr;
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
    return reinterpret_cast<void *>(gum_module_find_symbol_by_name(art, owned.c_str()));
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
    gum_module_enumerate_symbols(art, find_prefix, &search);
    return search.result;
}
}

extern "C" bool noa_lsplant_init(JNIEnv *env, void *handle) {
    auto init = reinterpret_cast<Init>(dlsym(handle, "_ZN7lsplant2v24InitEP7_JNIEnvRKNS0_8InitInfoE"));
    if (init == nullptr) return false;
    gum_init_embedded();
    interceptor = gum_interceptor_obtain();
    art = gum_process_find_module_by_name("libart.so");
    if (interceptor == nullptr || art == nullptr) return false;
    lsplant::InitInfo info{
        .inline_hooker = inline_hook,
        .inline_unhooker = inline_unhook,
        .art_symbol_resolver = resolve_symbol,
        .art_symbol_prefix_resolver = resolve_prefix,
    };
    return init(env, info);
}

extern "C" jobject noa_lsplant_hook(
    JNIEnv *env,
    void *handle,
    jobject target,
    jobject hooker,
    jobject callback
) {
    auto hook = reinterpret_cast<Hook>(dlsym(handle, "_ZN7lsplant2v24HookEP7_JNIEnvP8_jobjectS4_S4_"));
    return hook == nullptr ? nullptr : hook(env, target, hooker, callback);
}

extern "C" bool noa_lsplant_deoptimize(JNIEnv *env, void *handle, jobject target) {
    auto deoptimize = reinterpret_cast<Deoptimize>(dlsym(handle, "_ZN7lsplant2v210DeoptimizeEP7_JNIEnvP8_jobject"));
    return deoptimize != nullptr && deoptimize(env, target);
}
