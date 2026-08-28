package dev.noa.kakao;

import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.List;

/** Maps semantic roles to runtime classes, companions and singleton objects. */
final class KakaoClassResolver {
    private KakaoClassResolver() {}

    static Class<?> classFor(String role) throws Exception {
        if ("room-manager".equals(role)) {
            return largestOuter("ChatRoomListManager.kt");
        }
        if ("room-api".equals(role)) {
            return largestOuter("ChatRoomApiHelper.kt");
        }
        if ("member-repository".equals(role)) {
            return largestOuter("OpenChatMemberRepository.kt");
        }
        if ("open-link-manager".equals(role)) {
            return largestOuter("OlkManager.kt");
        }
        if ("open-profile-repository".equals(role)) {
            return largestOuter("OlkOpenProfileRepository.kt");
        }
        if ("open-link-connection".equals(role)) {
            for (Class<?> type : KakaoSignatureResolver.sourceClasses(
                    "ConnectionOpenLinkJoin.kt")) {
                for (Constructor<?> constructor : type.getDeclaredConstructors()) {
                    Class<?>[] parameters = constructor.getParameterTypes();
                    if (parameters.length == 1
                            && "android.content.Intent".equals(parameters[0].getName())) {
                        return type;
                    }
                }
            }
        }
        if ("open-link".equals(role)) {
            for (Class<?> type : KakaoSignatureResolver.sourceClasses("OpenLink.kt")) {
                if (type.getName().indexOf('$') < 0
                        && hasNoArgumentMethod(type, String.class)
                        && longGetterCount(type) >= 2) {
                    return type;
                }
            }
        }
        if ("open-chat-kakao-profile".equals(role)) {
            return profileWithFactory(String.class, String.class);
        }
        if ("open-chat-open-profile".equals(role)) {
            for (Class<?> type : KakaoSignatureResolver.sourceClasses("OpenLinkTypes.kt")) {
                for (Field field : type.getDeclaredFields()) {
                    if (!Modifier.isStatic(field.getModifiers())) {
                        continue;
                    }
                    if (hasFactory(field.getType(), long.class, true)) {
                        return type;
                    }
                }
            }
        }
        if ("open-chat-profile-use-type".equals(role)) {
            Class<?> profile = classFor("open-chat-open-profile");
            for (Class<?> nested : KakaoSignatureResolver.sourceClasses("OpenLinkTypes.kt")) {
                if (nested.isEnum() && nested.getName().startsWith(profile.getName() + "$")) {
                    return nested;
                }
            }
        }
        if ("sending-log-manager".equals(role)) {
            return largestOuter("ChatSendingLogManager.kt");
        }
        if ("sending-log-request".equals(role)) {
            return largestOuter("ChatSendingLogRequest.kt");
        }
        if ("sending-log-mode".equals(role)) {
            for (Class<?> type : KakaoSignatureResolver.sourceClasses(
                    "ChatSendingLogRequest.kt")) {
                if (!type.isEnum()) {
                    continue;
                }
                try {
                    @SuppressWarnings({"unchecked", "rawtypes"})
                    Object ignored = Enum.valueOf((Class) type, "Resend");
                    return type;
                } catch (Throwable ignored) {
                }
            }
        }
        throw new ClassNotFoundException("KakaoTalk class role was not found: " + role);
    }

    static Object objectFor(String role) throws Exception {
        Class<?> type = classFor(role);
        if ("room-api".equals(role)
                || "sending-log-request".equals(role)
                || "open-link".equals(role)
                || "open-link-connection".equals(role)
                || "open-chat-kakao-profile".equals(role)
                || "open-chat-open-profile".equals(role)) {
            Object companion = largestCompanion(type);
            if (companion != null) {
                return companion;
            }
        }
        List<Object> singletons = KakaoReflection.singletonObjects(type);
        if (!singletons.isEmpty()) {
            return singletons.get(0);
        }
        throw new NoSuchFieldException("singleton signature was not found for role: " + role);
    }

    static Object staticValueFor(String role, String typeName) throws Exception {
        Class<?> owner = classFor(role);
        Class<?> expected = Class.forName(
                typeName, false, KakaoSignatureResolver.appLoader());
        Object matched = null;
        for (Field field : owner.getDeclaredFields()) {
            if (!Modifier.isStatic(field.getModifiers())
                    || !expected.isAssignableFrom(field.getType())) {
                continue;
            }
            field.setAccessible(true);
            Object value = field.get(null);
            if (value == null) {
                continue;
            }
            if (matched != null) {
                throw new NoSuchFieldException(
                        "ambiguous static " + typeName + " for role: " + role);
            }
            matched = value;
        }
        if (matched == null) {
            throw new NoSuchFieldException(
                    "static " + typeName + " was not found for role: " + role);
        }
        return matched;
    }

    private static Object largestCompanion(Class<?> owner) {
        Object matched = null;
        int score = -1;
        for (Field field : owner.getDeclaredFields()) {
            if (!Modifier.isStatic(field.getModifiers())
                    || !field.getType().getName().startsWith(owner.getName() + "$")) {
                continue;
            }
            try {
                field.setAccessible(true);
                Object value = field.get(null);
                if (value == null) {
                    continue;
                }
                int candidate = value.getClass().getDeclaredMethods().length;
                if (candidate > score) {
                    matched = value;
                    score = candidate;
                }
            } catch (Throwable ignored) {
            }
        }
        return matched;
    }

    private static Class<?> largestOuter(String source) throws Exception {
        Class<?> matched = null;
        int score = -1;
        for (Class<?> type : KakaoSignatureResolver.sourceClasses(source)) {
            if (type.getName().indexOf('$') >= 0 || type.isInterface() || type.isEnum()) {
                continue;
            }
            int candidate = type.getDeclaredMethods().length * 4
                    + type.getDeclaredFields().length;
            if (candidate > score) {
                matched = type;
                score = candidate;
            }
        }
        if (matched == null) {
            throw new ClassNotFoundException("outer class signature was not found: " + source);
        }
        return matched;
    }

    private static Class<?> profileWithFactory(Class<?> first, Class<?> second) throws Exception {
        for (Class<?> type : KakaoSignatureResolver.sourceClasses("OpenLinkTypes.kt")) {
            for (Field field : type.getDeclaredFields()) {
                if (Modifier.isStatic(field.getModifiers())
                        && hasFactory(field.getType(), first, second)) {
                    return type;
                }
            }
        }
        throw new ClassNotFoundException("open-chat profile factory signature was not found");
    }

    private static boolean hasFactory(Class<?> type, Class<?> first, Class<?> second) {
        for (Method method : type.getDeclaredMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            if (!Modifier.isStatic(method.getModifiers())
                    && parameters.length == 2
                    && parameters[0] == first
                    && parameters[1] == second
                    && !method.getReturnType().isPrimitive()) {
                return true;
            }
        }
        return false;
    }

    private static boolean hasFactory(Class<?> type, Class<?> first, boolean secondIsEnum) {
        for (Method method : type.getDeclaredMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            if (!Modifier.isStatic(method.getModifiers())
                    && parameters.length == 2
                    && parameters[0] == first
                    && (!secondIsEnum || parameters[1].isEnum())
                    && !method.getReturnType().isPrimitive()) {
                return true;
            }
        }
        return false;
    }

    private static boolean hasNoArgumentMethod(Class<?> type, Class<?> result) {
        for (Method method : type.getDeclaredMethods()) {
            if (!Modifier.isStatic(method.getModifiers())
                    && method.getParameterTypes().length == 0
                    && method.getReturnType() == result) {
                return true;
            }
        }
        return false;
    }

    private static int longGetterCount(Class<?> type) {
        int count = 0;
        for (Method method : type.getDeclaredMethods()) {
            if (!Modifier.isStatic(method.getModifiers())
                    && method.getParameterTypes().length == 0
                    && (method.getReturnType() == long.class
                    || method.getReturnType() == Long.class)) {
                count++;
            }
        }
        return count;
    }
}
