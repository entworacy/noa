package dev.noa.kakao;

import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.List;

/** Resolves chat-room lookup, open-chat membership and kick relationships. */
final class KakaoRoomResolver {
    private static volatile RoomBinding roomBinding;
    private static volatile KickBinding kickBinding;

    private KakaoRoomResolver() {}

    static Object find(long roomId) throws Exception {
        RoomBinding cached = roomBinding;
        if (cached != null) {
            Object room = cached.lookup.invoke(cached.repository, roomId);
            if (room != null && KakaoReflection.hasLongValue(room, roomId)) return room;
        }
        synchronized (KakaoRoomResolver.class) {
            cached = discoverRoom(roomId);
            roomBinding = cached;
        }
        Object room = cached.lookup.invoke(cached.repository, roomId);
        if (room == null || !KakaoReflection.hasLongValue(room, roomId)) {
            throw new IllegalStateException("resolved chat room lookup failed for " + roomId);
        }
        return room;
    }

    static void kick(long roomId, long userId, long commandId) throws Exception {
        Object room = find(roomId);
        KickBinding cached = kickBinding;
        Object member = cached == null ? null : cached.member.invoke(cached.members, userId,
                cached.roomLink.invoke(room));
        if (member == null || !KakaoReflection.hasLongValue(member, userId)) {
            synchronized (KakaoRoomResolver.class) {
                cached = discoverKick(room, roomId, userId);
                kickBinding = cached;
            }
            member = cached.member.invoke(cached.members, userId, cached.roomLink.invoke(room));
        }
        if (member == null || !KakaoReflection.hasLongValue(member, userId)) {
            throw new IllegalStateException("open chat member not found: " + userId);
        }
        Object listener = KakaoCallbacks.completion(
                KakaoSignatureResolver.appLoader(), cached.listener, commandId);
        cached.kick.invoke(cached.foreground, room, member, false, false, listener);
    }

    private static RoomBinding discoverRoom(long roomId) throws Exception {
        List<String> failures = new ArrayList<>();
        for (Class<?> type : KakaoSignatureResolver.sourceClasses("ChatRoomListManager.kt")) {
            if (type.isInterface() || type.isEnum() || type.getDeclaredMethods().length < 20) {
                continue;
            }
            for (Object repository : KakaoReflection.singletonObjects(type)) {
                for (Method method : type.getDeclaredMethods()) {
                    Class<?>[] parameters = method.getParameterTypes();
                    if (Modifier.isStatic(method.getModifiers())
                            || parameters.length != 1 || parameters[0] != long.class
                            || method.getReturnType().isPrimitive()
                            || method.getReturnType() == String.class
                            || method.getReturnType().isArray()) continue;
                    try {
                        method.setAccessible(true);
                        Object room = method.invoke(repository, roomId);
                        if (room != null && KakaoReflection.hasLongValue(room, roomId)) {
                            return new RoomBinding(repository, method);
                        }
                    } catch (Throwable error) {
                        KakaoReflection.remember(
                                failures, type.getName() + "." + method.getName(), error);
                    }
                }
            }
        }
        throw new NoSuchMethodException("chat-room signature was not found: "
                + KakaoReflection.lastFailure(failures));
    }

    private static KickBinding discoverKick(Object room, long roomId, long userId)
            throws Exception {
        List<KakaoReflection.LongValue> roomValues = KakaoReflection.longValues(room);
        List<String> failures = new ArrayList<>();
        MemberMatch memberMatch = null;
        for (Class<?> type : KakaoSignatureResolver.sourceClasses(
                "OpenChatMemberRepository.kt")) {
            for (Object repository : KakaoReflection.singletonObjects(type)) {
                for (Method method : type.getDeclaredMethods()) {
                    Class<?>[] parameters = method.getParameterTypes();
                    if (Modifier.isStatic(method.getModifiers())
                            || parameters.length != 2
                            || parameters[0] != long.class || parameters[1] != long.class
                            || method.getReturnType().isPrimitive()
                            || method.getReturnType() == Object.class) continue;
                    method.setAccessible(true);
                    for (KakaoReflection.LongValue value : roomValues) {
                        if (value.value <= 0 || value.value == roomId) continue;
                        try {
                            Object member = method.invoke(repository, userId, value.value);
                            if (member != null
                                    && KakaoReflection.hasLongValue(member, userId)) {
                                memberMatch = new MemberMatch(
                                        repository, method, value.method, member);
                                break;
                            }
                        } catch (Throwable error) {
                            KakaoReflection.remember(
                                    failures, type.getName() + "." + method.getName(), error);
                        }
                    }
                    if (memberMatch != null) break;
                }
                if (memberMatch != null) break;
            }
            if (memberMatch != null) break;
        }
        if (memberMatch == null) {
            throw new NoSuchMethodException("open-chat member signature was not found: "
                    + KakaoReflection.lastFailure(failures));
        }

        for (Class<?> type : KakaoSignatureResolver.sourceClasses("OlkManager.kt")) {
            for (Object manager : KakaoReflection.singletonObjects(type)) {
                for (Method accessor : type.getDeclaredMethods()) {
                    if (Modifier.isStatic(accessor.getModifiers())
                            || accessor.getParameterTypes().length != 0
                            || accessor.getReturnType().isPrimitive()
                            || accessor.getReturnType() == String.class) continue;
                    Method kick = kickMethod(
                            accessor.getReturnType(), room.getClass(), memberMatch.value.getClass());
                    if (kick == null) continue;
                    try {
                        accessor.setAccessible(true);
                        Object foreground = accessor.invoke(manager);
                        if (foreground == null) continue;
                        Class<?> listener = kick.getParameterTypes()[4];
                        if (!listener.isInterface()) continue;
                        memberMatch.method.setAccessible(true);
                        memberMatch.roomLink.setAccessible(true);
                        kick.setAccessible(true);
                        return new KickBinding(memberMatch.repository, memberMatch.method,
                                memberMatch.roomLink, foreground, kick, listener);
                    } catch (Throwable error) {
                        KakaoReflection.remember(
                                failures, type.getName() + "." + accessor.getName(), error);
                    }
                }
            }
        }
        throw new NoSuchMethodException("open-chat kick signature was not found: "
                + KakaoReflection.lastFailure(failures));
    }

    private static Method kickMethod(Class<?> foreground, Class<?> room, Class<?> member) {
        Method matched = null;
        for (Method method : KakaoReflection.allMethods(foreground)) {
            Class<?>[] parameters = method.getParameterTypes();
            if (Modifier.isStatic(method.getModifiers())
                    || method.getReturnType() != void.class
                    || parameters.length != 5
                    || !parameters[0].isAssignableFrom(room)
                    || !parameters[1].isAssignableFrom(member)
                    || parameters[2] != boolean.class || parameters[3] != boolean.class
                    || !parameters[4].isInterface()) continue;
            if (matched != null) return null;
            matched = method;
        }
        return matched;
    }

    private static final class RoomBinding {
        final Object repository;
        final Method lookup;

        RoomBinding(Object repository, Method lookup) {
            this.repository = repository;
            this.lookup = lookup;
        }
    }

    private static final class MemberMatch {
        final Object repository;
        final Method method;
        final Method roomLink;
        final Object value;

        MemberMatch(Object repository, Method method, Method roomLink, Object value) {
            this.repository = repository;
            this.method = method;
            this.roomLink = roomLink;
            this.value = value;
        }
    }

    private static final class KickBinding {
        final Object members;
        final Method member;
        final Method roomLink;
        final Object foreground;
        final Method kick;
        final Class<?> listener;

        KickBinding(Object members, Method member, Method roomLink, Object foreground,
                Method kick, Class<?> listener) {
            this.members = members;
            this.member = member;
            this.roomLink = roomLink;
            this.foreground = foreground;
            this.kick = kick;
            this.listener = listener;
        }
    }
}
