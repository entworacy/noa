package dev.noa.kakao;

import java.lang.reflect.Method;
import java.lang.reflect.Modifier;

/** Resolves the native kicked-member removal operation (KLDELITEM). */
final class KakaoUnbanResolver {
    private KakaoUnbanResolver() {}

    static void unban(long linkId, long roomId, long userId, final long commandId)
            throws Exception {
        if (linkId <= 0 || roomId <= 0 || userId <= 0) {
            throw new IllegalArgumentException("positive link, room and user IDs are required");
        }
        Object manager = KakaoClassResolver.objectFor("open-link-manager");
        Method accessor = null;
        Method unban = null;
        for (Method method : manager.getClass().getDeclaredMethods()) {
            if (Modifier.isStatic(method.getModifiers()) || method.isBridge()
                    || method.isSynthetic() || method.getReturnType().isInterface()
                    || method.getParameterTypes().length != 0) continue;
            Method candidate = unbanMethod(method.getReturnType());
            if (candidate == null) continue;
            if (accessor != null) {
                throw new NoSuchMethodException("ambiguous open-chat unban accessor");
            }
            accessor = method;
            unban = candidate;
        }
        if (unban == null) {
            throw new NoSuchMethodException("open-chat unban signature was not found");
        }
        accessor.setAccessible(true);
        unban.setAccessible(true);
        Object foreground = accessor.invoke(manager);
        if (foreground == null) throw new IllegalStateException("open-chat unban target is null");
        // The kicked-member list passes the member's user ID as KLDELITEM.kid.
        unban.invoke(foreground, linkId, roomId, userId, new Runnable() {
            @Override public void run() { Bridge.complete(commandId, true, null); }
        });
    }

    private static Method unbanMethod(Class<?> type) {
        Method matched = null;
        for (Method method : KakaoReflection.allMethods(type)) {
            Class<?>[] parameters = method.getParameterTypes();
            if (Modifier.isStatic(method.getModifiers()) || method.getReturnType() != void.class
                    || parameters.length != 4 || parameters[0] != long.class
                    || parameters[1] != long.class || parameters[2] != long.class
                    || parameters[3] != Runnable.class) continue;
            if (matched != null) return null;
            matched = method;
        }
        return matched;
    }
}
