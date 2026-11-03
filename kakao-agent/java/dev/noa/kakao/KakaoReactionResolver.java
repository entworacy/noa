package dev.noa.kakao;

import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.lang.reflect.Proxy;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.UUID;

/** Uses the same asynchronous reaction operations as the message reaction picker. */
final class KakaoReactionResolver {
    private KakaoReactionResolver() {}

    static void react(long room, long log, long link, final long command,
            int reactionType, int emoticonType, String emoticonId) throws Exception {
        if (room <= 0 || log <= 0 || link < 0) throw new IllegalArgumentException("invalid target IDs");
        boolean emoticon = emoticonId != null;
        if ((!emoticon && (reactionType < 1 || reactionType > 6))
                || (emoticon && (reactionType != 0 || emoticonType < 1 || emoticonType > 2
                    || emoticonId.trim().isEmpty()))) throw new IllegalArgumentException("invalid reaction");
        Object manager = KakaoClassResolver.objectFor("reaction-manager");
        Method selected = null;
        for (Method method : manager.getClass().getDeclaredMethods()) {
            if (Modifier.isStatic(method.getModifiers()) || method.getReturnType() != void.class
                    || method.isSynthetic() || method.isBridge()) continue;
            Class<?>[] p = method.getParameterTypes();
            boolean match = !emoticon
                ? p.length == 5 && p[0] == int.class && p[1] == long.class && p[2] == long.class
                    && p[3] == Long.class && p[4].getName().equals("kotlin.jvm.functions.Function0")
                : p.length == 7 && p[1] == long.class && p[2] == long.class && p[3] == Long.class
                    && p[4] == long.class && p[5] == String.class
                    && p[6].getName().equals("kotlin.jvm.functions.Function0");
            if (!match) continue;
            if (selected != null) throw new NoSuchMethodException("ambiguous reaction operation");
            selected = method;
        }
        if (selected == null) throw new NoSuchMethodException("reaction operation not found");
        Class<?> callbackType = selected.getParameterTypes()[emoticon ? 6 : 4];
        Object callback = Proxy.newProxyInstance(callbackType.getClassLoader(), new Class<?>[]{callbackType},
            (proxy, method, args) -> {
                if (method.getDeclaringClass() == Object.class) {
                    if (method.getName().equals("toString")) return "NoaReactionCallback";
                    if (method.getName().equals("hashCode")) return System.identityHashCode(proxy);
                    if (method.getName().equals("equals")) return proxy == args[0];
                }
                if (!method.getName().equals("invoke") || method.getParameterTypes().length != 0)
                    throw new IllegalStateException("unexpected reaction callback");
                Bridge.complete(command, true, null);
                return KakaoReflection.singletonObjects(
                    callbackType.getClassLoader().loadClass("kotlin.Unit")).get(0);
            });
        Long linkId = link == 0 ? null : Long.valueOf(link);
        selected.setAccessible(true);
        if (!emoticon) {
            selected.invoke(manager, reactionType, room, log, linkId, callback);
        } else {
            Class<?> item = callbackType.getClassLoader().loadClass(
                    "com.kakao.talk.module.emoticon.data.EmoticonReaction");
            Object reaction = item.getConstructor(int.class, String.class).newInstance(emoticonType, emoticonId);
            Object params = selected.getParameterTypes()[0].getConstructor(List.class, List.class, List.class)
                    .newInstance(Arrays.asList(reaction), Collections.emptyList(), Collections.emptyList());
            selected.invoke(manager, params, room, log, linkId, System.currentTimeMillis(),
                    UUID.randomUUID().toString(), callback);
        }
    }
}
