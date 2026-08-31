package dev.noa.kakao;

import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.List;

/** Resolves the current LOCO StateFlow without renamed fields or classes. */
final class KakaoLocoStateResolver {
    private static volatile Binding binding;

    private KakaoLocoStateResolver() {}

    static boolean connected() throws Exception {
        Binding cached = resolve();
        Object flow = cached.accessor.invoke(cached.singleton);
        Object value = cached.value.invoke(flow);
        return value != null && "connected".equalsIgnoreCase(String.valueOf(value));
    }

    static String describe() throws Exception {
        Binding cached = resolve();
        return cached.accessor.getDeclaringClass().getName() + "." + cached.accessor.getName();
    }

    private static Binding resolve() throws Exception {
        Binding cached = binding;
        if (cached != null) return cached;
        synchronized (KakaoLocoStateResolver.class) {
            cached = binding;
            if (cached == null) {
                cached = discover();
                binding = cached;
            }
        }
        return cached;
    }

    private static Binding discover() throws Exception {
        List<Class<?>> stateTypes = KakaoSignatureResolver.sourceClasses("LocoState.kt");
        for (Class<?> type : KakaoSignatureResolver.sourceClasses("Loco.kt")) {
            if (type.getName().indexOf('$') >= 0 || type.isInterface()) continue;
            for (Object singleton : KakaoReflection.singletonObjects(type)) {
                for (Method accessor : type.getDeclaredMethods()) {
                    if (Modifier.isStatic(accessor.getModifiers())
                            || accessor.getParameterTypes().length != 0
                            || accessor.getReturnType().isPrimitive()) continue;
                    try {
                        Method value = accessor.getReturnType().getMethod("getValue");
                        if (value.getParameterTypes().length != 0) continue;
                        accessor.setAccessible(true);
                        value.setAccessible(true);
                        Object flow = accessor.invoke(singleton);
                        if (flow == null) continue;
                        Object current = value.invoke(flow);
                        if (current != null
                                && KakaoReflection.matchesAnyType(current, stateTypes)) {
                            return new Binding(singleton, accessor, value);
                        }
                    } catch (Throwable ignored) {
                    }
                }
            }
        }
        throw new NoSuchMethodException("LOCO state-flow signature was not found");
    }

    private static final class Binding {
        final Object singleton;
        final Method accessor;
        final Method value;

        Binding(Object singleton, Method accessor, Method value) {
            this.singleton = singleton;
            this.accessor = accessor;
            this.value = value;
        }
    }
}
