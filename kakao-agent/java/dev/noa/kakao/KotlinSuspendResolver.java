package dev.noa.kakao;

import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.lang.reflect.ParameterizedType;
import java.lang.reflect.Type;
import java.lang.reflect.WildcardType;
import java.util.ArrayList;
import java.util.List;

/** Resolves Kotlin suspend methods whose erased JVM signatures are otherwise identical. */
final class KotlinSuspendResolver {
    private static final String CONTINUATION = "kotlin.coroutines.Continuation";

    private KotlinSuspendResolver() {}

    static Method uniqueInstanceMethodReturning(
            Class<?> target, String operation, String resultType) throws NoSuchMethodException {
        List<Method> candidates = new ArrayList<>();
        for (Method method : target.getDeclaredMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            if (!Modifier.isStatic(method.getModifiers())
                    && parameters.length == 1
                    && CONTINUATION.equals(parameters[0].getName())
                    && method.getReturnType() == Object.class) {
                candidates.add(method);
            }
        }
        if (candidates.size() == 1) {
            return candidates.get(0);
        }

        Method matched = null;
        for (Method method : candidates) {
            if (!continuationResultIs(method, resultType)) {
                continue;
            }
            if (matched != null) {
                throw new NoSuchMethodException(
                        "ambiguous " + resultType + "-returning " + operation + " signature");
            }
            matched = method;
        }
        if (matched != null) {
            return matched;
        }
        if (!candidates.isEmpty()) {
            throw new NoSuchMethodException(
                    "ambiguous " + operation + " signature: " + candidates);
        }
        return null;
    }

    private static boolean continuationResultIs(Method method, String expectedType) {
        Type[] parameters = method.getGenericParameterTypes();
        if (parameters.length != 1 || !(parameters[0] instanceof ParameterizedType)) {
            return false;
        }
        ParameterizedType continuation = (ParameterizedType) parameters[0];
        Type raw = continuation.getRawType();
        if (!(raw instanceof Class) || !CONTINUATION.equals(((Class<?>) raw).getName())) {
            return false;
        }
        Type[] arguments = continuation.getActualTypeArguments();
        return arguments.length == 1 && typeIs(arguments[0], expectedType);
    }

    private static boolean typeIs(Type type, String expectedType) {
        if (type instanceof Class) {
            return expectedType.equals(((Class<?>) type).getName());
        }
        if (type instanceof WildcardType) {
            WildcardType wildcard = (WildcardType) type;
            for (Type bound : wildcard.getLowerBounds()) {
                if (typeIs(bound, expectedType)) {
                    return true;
                }
            }
            for (Type bound : wildcard.getUpperBounds()) {
                if (typeIs(bound, expectedType)) {
                    return true;
                }
            }
        }
        return false;
    }
}
