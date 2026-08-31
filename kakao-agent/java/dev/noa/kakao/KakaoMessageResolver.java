package dev.noa.kakao;

import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.Collections;

/** Resolves KakaoTalk's open-chat manager message-hiding relationship. */
final class KakaoMessageResolver {
    private static volatile HideBinding hideBinding;
    private static volatile Method feedTypeConverter;

    private KakaoMessageResolver() {}

    static void hide(long roomId, long logId, int logType, long commandId, String message)
            throws Exception {
        Object room = KakaoRoomResolver.find(roomId);
        HideBinding binding = hideBinding;
        if (binding == null || !binding.roomType.isInstance(room)) {
            synchronized (KakaoMessageResolver.class) {
                binding = discover(room, roomId);
                hideBinding = binding;
            }
        }
        long linkId = ((Number) binding.roomLink.invoke(room)).longValue();
        if (linkId <= 0) {
            throw new IllegalStateException("open-chat link ID is invalid: " + linkId);
        }
        Object feedType = resolveFeedType(message == null ? "" : message);
        binding.hide.invoke(
                binding.foreground,
                linkId,
                room,
                Collections.singletonList(logId),
                Collections.singletonList(logType),
                Collections.singletonList(feedType));
        // KakaoTalk's public foreground method schedules the LOCO request and has
        // no completion callback. Completion here means it accepted the request.
        Bridge.complete(commandId, true, null);
    }

    private static HideBinding discover(Object room, long roomId) throws Exception {
        KakaoReflection.LongValue link = findLink(room, roomId);
        for (Class<?> type : KakaoSignatureResolver.sourceClasses("OlkManager.kt")) {
            for (Object manager : KakaoReflection.singletonObjects(type)) {
                for (Method accessor : type.getDeclaredMethods()) {
                    if (Modifier.isStatic(accessor.getModifiers())
                            || accessor.getParameterTypes().length != 0
                            || accessor.getReturnType().isPrimitive()
                            || accessor.getReturnType() == String.class) {
                        continue;
                    }
                    Method hide = hideMethod(accessor.getReturnType(), room.getClass());
                    if (hide == null) continue;
                    accessor.setAccessible(true);
                    Object foreground = accessor.invoke(manager);
                    if (foreground == null) continue;
                    link.method.setAccessible(true);
                    hide.setAccessible(true);
                    return new HideBinding(
                            room.getClass(), link.method, foreground, hide);
                }
            }
        }
        throw new NoSuchMethodException("open-chat message hide signature was not found");
    }

    private static KakaoReflection.LongValue findLink(Object room, long roomId)
            throws Exception {
        KakaoReflection.LongValue matched = null;
        for (KakaoReflection.LongValue candidate : KakaoReflection.longValues(room)) {
            if (candidate.value <= 0 || candidate.value == roomId) continue;
            Object linkedRoom;
            try {
                linkedRoom = KakaoOpenChatResolver.findRoom(candidate.value);
            } catch (Throwable ignored) {
                continue;
            }
            if (linkedRoom == null || !KakaoReflection.hasLongValue(linkedRoom, roomId)) continue;
            if (matched != null && matched.value != candidate.value) {
                throw new NoSuchMethodException("ambiguous open-chat link ID signature");
            }
            matched = candidate;
        }
        if (matched == null) {
            throw new NoSuchMethodException("open-chat link ID signature was not found");
        }
        return matched;
    }

    private static Method hideMethod(Class<?> foreground, Class<?> room) {
        Method matched = null;
        for (Method method : KakaoReflection.allMethods(foreground)) {
            Class<?>[] parameters = method.getParameterTypes();
            if (Modifier.isStatic(method.getModifiers())
                    || method.getReturnType() != void.class
                    || parameters.length != 5
                    || parameters[0] != long.class
                    || !parameters[1].isAssignableFrom(room)
                    || !java.util.List.class.isAssignableFrom(parameters[2])
                    || !java.util.List.class.isAssignableFrom(parameters[3])
                    || !java.util.List.class.isAssignableFrom(parameters[4])) {
                continue;
            }
            if (matched != null) return null;
            matched = method;
        }
        return matched;
    }

    private static Object resolveFeedType(String message) throws Exception {
        Method cached = feedTypeConverter;
        if (cached == null) {
            synchronized (KakaoMessageResolver.class) {
                cached = discoverFeedTypeConverter();
                feedTypeConverter = cached;
            }
        }
        return cached.invoke(null, message);
    }

    private static Method discoverFeedTypeConverter() throws Exception {
        Method matched = null;
        for (Class<?> type : KakaoSignatureResolver.sourceClasses("FeedType.kt")) {
            if (!type.isEnum()) continue;
            for (Method method : type.getDeclaredMethods()) {
                Class<?>[] parameters = method.getParameterTypes();
                if (!Modifier.isStatic(method.getModifiers())
                        || "valueOf".equals(method.getName())
                        || parameters.length != 1 || parameters[0] != String.class
                        || method.getReturnType() != type) {
                    continue;
                }
                if (matched != null) {
                    throw new NoSuchMethodException("ambiguous FeedType converter signature");
                }
                matched = method;
            }
        }
        if (matched == null) {
            throw new NoSuchMethodException("FeedType converter signature was not found");
        }
        matched.setAccessible(true);
        return matched;
    }

    private static final class HideBinding {
        final Class<?> roomType;
        final Method roomLink;
        final Object foreground;
        final Method hide;

        HideBinding(Class<?> roomType, Method roomLink, Object foreground, Method hide) {
            this.roomType = roomType;
            this.roomLink = roomLink;
            this.foreground = foreground;
            this.hide = hide;
        }
    }
}
