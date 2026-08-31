package dev.noa.kakao;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/** Shared reflection primitives used by the domain-specific resolvers. */
final class KakaoReflection {
    private KakaoReflection() {}

    static boolean matchesAnyType(Object value, List<Class<?>> types) {
        for (Class<?> type : types) {
            if (type.isInstance(value)) return true;
        }
        return false;
    }

    static boolean hasLongValue(Object target, long expected) {
        try {
            for (LongValue value : longValues(target)) {
                if (value.value == expected) return true;
            }
        } catch (Throwable ignored) {
        }
        return false;
    }

    static List<LongValue> longValues(Object target) {
        List<LongValue> values = new ArrayList<>();
        for (Method method : allMethods(target.getClass())) {
            if (Modifier.isStatic(method.getModifiers())
                    || method.getParameterTypes().length != 0
                    || (method.getReturnType() != long.class
                    && method.getReturnType() != Long.class)) {
                continue;
            }
            try {
                method.setAccessible(true);
                Object result = method.invoke(target);
                if (result instanceof Number) {
                    values.add(new LongValue(method, ((Number) result).longValue()));
                }
            } catch (Throwable ignored) {
            }
        }
        return values;
    }

    static List<Method> allMethods(Class<?> type) {
        Map<String, Method> methods = new LinkedHashMap<>();
        for (Class<?> current = type; current != null; current = current.getSuperclass()) {
            for (Method method : current.getDeclaredMethods()) {
                String key = method.getName() + Arrays.toString(method.getParameterTypes());
                if (!methods.containsKey(key)) methods.put(key, method);
            }
        }
        for (Class<?> contract : type.getInterfaces()) {
            for (Method method : contract.getMethods()) {
                String key = method.getName() + Arrays.toString(method.getParameterTypes());
                if (!methods.containsKey(key)) methods.put(key, method);
            }
        }
        return new ArrayList<>(methods.values());
    }

    static List<Object> singletonObjects(Class<?> type) {
        List<Object> values = new ArrayList<>();
        for (Field field : type.getDeclaredFields()) {
            if (!Modifier.isStatic(field.getModifiers())) continue;
            Class<?> fieldType = field.getType();
            if (!type.isAssignableFrom(fieldType)
                    && !fieldType.getName().startsWith(type.getName() + "$")) {
                continue;
            }
            try {
                field.setAccessible(true);
                Object value = field.get(null);
                if (value == null) continue;
                if (type.isInstance(value)) {
                    values.add(value);
                    continue;
                }
                for (Method method : value.getClass().getDeclaredMethods()) {
                    if (Modifier.isStatic(method.getModifiers())
                            || method.getParameterTypes().length != 0
                            || !type.isAssignableFrom(method.getReturnType())) {
                        continue;
                    }
                    method.setAccessible(true);
                    Object singleton = method.invoke(value);
                    if (singleton != null) values.add(singleton);
                }
            } catch (Throwable ignored) {
            }
        }
        return values;
    }

    static void remember(List<String> failures, String target, Throwable error) {
        if (failures.size() < 24) failures.add(target + ": " + error);
    }

    static String lastFailure(List<String> failures) {
        return failures.isEmpty() ? "no candidate matched"
                : failures.get(failures.size() - 1);
    }

    static final class LongValue {
        final Method method;
        final long value;

        LongValue(Method method, long value) {
            this.method = method;
            this.value = value;
        }
    }
}
