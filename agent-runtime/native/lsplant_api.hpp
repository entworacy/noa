#pragma once

#include <functional>
#include <jni.h>
#include <string_view>

namespace lsplant {
inline namespace v2 {
struct InitInfo {
    using InlineHookFunType = std::function<void *(void *, void *)>;
    using InlineUnhookFunType = std::function<bool(void *)>;
    using ArtSymbolResolver = std::function<void *(std::string_view)>;
    using ArtSymbolPrefixResolver = std::function<void *(std::string_view)>;

    InlineHookFunType inline_hooker;
    InlineUnhookFunType inline_unhooker;
    ArtSymbolResolver art_symbol_resolver;
    ArtSymbolPrefixResolver art_symbol_prefix_resolver;
    std::string_view generated_class_name = "NoaHook_";
    std::string_view generated_source_name = "Noa";
    std::string_view generated_field_name = "hooker";
    std::string_view generated_method_name = "{target}";
};
}
}
