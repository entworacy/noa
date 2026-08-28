package dev.noa.kakao;

import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

/** Selects runtime methods from structural and DEX call-graph signatures. */
final class KakaoOperationResolver {
    private static final Map<String, Method> CACHE = new ConcurrentHashMap<>();

    private KakaoOperationResolver() {}

    static Object invoke(String operation, Object target, Object[] arguments) throws Exception {
        if (target == null) {
            throw new NullPointerException(operation + " target is null");
        }
        Object[] actual = arguments == null ? new Object[0] : arguments;
        String cacheKey = operation + "@" + target.getClass().getName();
        Method method = CACHE.get(cacheKey);
        if (method == null || !argumentsMatch(method, actual)) {
            method = resolve(operation, target.getClass(), actual);
            method.setAccessible(true);
            CACHE.put(cacheKey, method);
        }
        return method.invoke(target, actual);
    }

    static String verify(String operation, String targetRole) throws Exception {
        Object target = KakaoClassResolver.objectFor(targetRole);
        List<KakaoSignatureIndex.MethodRef> refs = KakaoSignatureResolver.operationRefs(operation);
        if (refs == null || refs.isEmpty()) {
            throw new NoSuchMethodException("DEX call-graph signature was not found: " + operation);
        }
        Set<String> matchedNames = new LinkedHashSet<>();
        int overloads = 0;
        for (KakaoSignatureIndex.MethodRef ref : refs) {
            if (!ref.owner.equals(target.getClass().getName())) {
                continue;
            }
            for (Method method : target.getClass().getDeclaredMethods()) {
                if (!Modifier.isStatic(method.getModifiers())
                        && method.getName().equals(ref.name)) {
                    matchedNames.add(method.getName());
                    overloads++;
                }
            }
        }
        if (matchedNames.isEmpty()) {
            throw new NoSuchMethodException(
                    "call-graph target does not match runtime object: " + operation);
        }
        return operation + "=" + target.getClass().getName()
                + matchedNames + "(" + overloads + " overloads)";
    }

    private static Method resolve(
            String operation, Class<?> target, Object[] arguments) throws Exception {
        if ("get-members".equals(operation)) {
            Method matched = null;
            for (Method method : target.getDeclaredMethods()) {
                Class<?>[] parameters = method.getParameterTypes();
                if (Modifier.isStatic(method.getModifiers())
                        || parameters.length != 3
                        || parameters[0] != long.class
                        || !List.class.isAssignableFrom(parameters[1])
                        || !"kotlin.coroutines.Continuation".equals(parameters[2].getName())
                        || method.getReturnType() != Object.class) {
                    continue;
                }
                if (matched != null) {
                    throw new NoSuchMethodException("ambiguous get-members signature");
                }
                matched = method;
            }
            if (matched != null) {
                return matched;
            }
        }
        if ("load-sending-log".equals(operation)) {
            Method matched = null;
            for (Method method : target.getDeclaredMethods()) {
                Class<?>[] parameters = method.getParameterTypes();
                if (Modifier.isStatic(method.getModifiers())
                        || parameters.length != 1
                        || !"kotlin.coroutines.Continuation".equals(parameters[0].getName())
                        || method.getReturnType() != Object.class) {
                    continue;
                }
                if (matched != null) {
                    throw new NoSuchMethodException("ambiguous sending-log load signature");
                }
                matched = method;
            }
            if (matched != null) {
                return matched;
            }
        }

        Class<?> openLink = null;
        if (operation.startsWith("open-link-")
                || "cached-open-link".equals(operation)
                || "apply-open-profile".equals(operation)
                || "create-open-chat".equals(operation)) {
            openLink = KakaoClassResolver.classFor("open-link");
        }
        Method shaped = null;
        for (Method method : target.getDeclaredMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            boolean match = false;
            if ("cached-open-link".equals(operation)) {
                match = !Modifier.isStatic(method.getModifiers())
                        && parameters.length == 1 && parameters[0] == long.class
                        && openLink.isAssignableFrom(method.getReturnType());
            } else if ("open-link-join-intent".equals(operation)) {
                match = !Modifier.isStatic(method.getModifiers())
                        && parameters.length == 2
                        && parameters[0] == String.class && parameters[1] == String.class
                        && "android.content.Intent".equals(method.getReturnType().getName());
            } else if ("open-link-connection-response".equals(operation)) {
                match = !Modifier.isStatic(method.getModifiers())
                        && parameters.length == 0 && !method.getReturnType().isPrimitive()
                        && method.getReturnType() != String.class;
            } else if ("open-link-convert".equals(operation)) {
                match = !Modifier.isStatic(method.getModifiers())
                        && parameters.length == 1
                        && openLink.isAssignableFrom(method.getReturnType());
            } else if ("open-link-cache".equals(operation)) {
                match = Modifier.isPublic(method.getModifiers())
                        && !Modifier.isStatic(method.getModifiers())
                        && method.getReturnType() == void.class
                        && parameters.length == 1 && parameters[0] == openLink;
            } else if ("apply-open-profile".equals(operation)) {
                match = Modifier.isPublic(method.getModifiers())
                        && !Modifier.isStatic(method.getModifiers())
                        && method.getReturnType() == void.class
                        && parameters.length == 2 && parameters[0] == openLink
                        && arguments.length == 2 && arguments[1] != null
                        && parameters[1].isInstance(arguments[1]);
            } else if ("create-open-chat".equals(operation)) {
                match = !Modifier.isStatic(method.getModifiers())
                        && parameters.length == 4 && parameters[0] == openLink
                        && parameters[1] == String.class && parameters[2] == String.class
                        && parameters[3] == String.class
                        && !method.getReturnType().isPrimitive();
            } else if ("create-kakao-profile".equals(operation)) {
                match = !Modifier.isStatic(method.getModifiers())
                        && parameters.length == 2
                        && parameters[0] == String.class && parameters[1] == String.class
                        && !method.getReturnType().isPrimitive();
            } else if ("create-open-profile".equals(operation)) {
                match = !Modifier.isStatic(method.getModifiers())
                        && parameters.length == 2 && parameters[0] == long.class
                        && parameters[1].isEnum() && !method.getReturnType().isPrimitive();
            }
            if (!match || !argumentsMatch(method, arguments)) {
                continue;
            }
            if (shaped != null) {
                throw new NoSuchMethodException("ambiguous " + operation + " signature");
            }
            shaped = method;
        }
        if (shaped != null) {
            return shaped;
        }

        List<KakaoSignatureIndex.MethodRef> refs = KakaoSignatureResolver.operationRefs(operation);
        if (refs != null) {
            for (KakaoSignatureIndex.MethodRef ref : refs) {
                if (!ref.owner.equals(target.getName())) {
                    continue;
                }
                for (Method method : target.getDeclaredMethods()) {
                    if (method.getName().equals(ref.name)
                            && argumentsMatch(method, arguments)) {
                        return method;
                    }
                }
            }
        }
        throw new NoSuchMethodException(
                "operation signature was not found: " + operation + " on " + target.getName());
    }

    private static boolean argumentsMatch(Method method, Object[] arguments) {
        Class<?>[] parameters = method.getParameterTypes();
        if (Modifier.isStatic(method.getModifiers()) || parameters.length != arguments.length) {
            return false;
        }
        for (int index = 0; index < parameters.length; index++) {
            Object argument = arguments[index];
            if (argument == null) {
                if (parameters[index].isPrimitive()) {
                    return false;
                }
                continue;
            }
            Class<?> expected = boxed(parameters[index]);
            if (!expected.isInstance(argument)) {
                return false;
            }
        }
        return true;
    }

    private static Class<?> boxed(Class<?> type) {
        if (!type.isPrimitive()) return type;
        if (type == boolean.class) return Boolean.class;
        if (type == byte.class) return Byte.class;
        if (type == short.class) return Short.class;
        if (type == int.class) return Integer.class;
        if (type == long.class) return Long.class;
        if (type == float.class) return Float.class;
        if (type == double.class) return Double.class;
        if (type == char.class) return Character.class;
        return type;
    }
}
