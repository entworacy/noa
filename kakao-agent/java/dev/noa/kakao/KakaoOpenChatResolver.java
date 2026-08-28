package dev.noa.kakao;

import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.Collections;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Set;

/** Resolves OpenLink models, IDs, profiles and related chat rooms. */
final class KakaoOpenChatResolver {
    private KakaoOpenChatResolver() {}

    static String openProfileUrl(long linkId) throws Exception {
        Object manager = KakaoSignatureResolver.objectFor("open-link-manager");
        Object cached = KakaoSignatureResolver.invokeOperation(
                "cached-open-link", manager, new Object[] {linkId});
        String url = findOpenProfileUrl(cached, 1,
                Collections.newSetFromMap(new IdentityHashMap<Object, Boolean>()));
        if (url != null) return url;
        Object repository = KakaoSignatureResolver.objectFor("open-profile-repository");
        Method request = null;
        for (Method method : repository.getClass().getDeclaredMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            if (!Modifier.isStatic(method.getModifiers())
                    && parameters.length == 2 && parameters[0] == long.class
                    && "org.json.JSONObject".equals(parameters[1].getName())
                    && !method.getReturnType().isPrimitive()) {
                if (request != null) {
                    throw new NoSuchMethodException("ambiguous open-profile request signature");
                }
                request = method;
            }
        }
        if (request == null) {
            throw new NoSuchMethodException("open-profile request signature was not found");
        }
        request.setAccessible(true);
        Object response = request.invoke(repository, linkId, null);
        url = findOpenProfileUrl(response, 3,
                Collections.newSetFromMap(new IdentityHashMap<Object, Boolean>()));
        if (url == null) {
            throw new IllegalStateException("open profile URL was not found: " + linkId);
        }
        return url;
    }

    static Object convertOpenLink(Object response) throws Exception {
        if (response == null) {
            throw new NullPointerException("open-link connection response is null");
        }
        Object companion = KakaoSignatureResolver.objectFor("open-link");
        Class<?> openLink = KakaoSignatureResolver.classFor("open-link");
        for (Method getter : KakaoReflection.allMethods(response.getClass())) {
            if (Modifier.isStatic(getter.getModifiers())
                    || getter.getParameterTypes().length != 0
                    || getter.getReturnType().isPrimitive()
                    || getter.getReturnType() == String.class) continue;
            for (Method converter : companion.getClass().getDeclaredMethods()) {
                Class<?>[] parameters = converter.getParameterTypes();
                if (Modifier.isStatic(converter.getModifiers()) || parameters.length != 1
                        || !parameters[0].isAssignableFrom(getter.getReturnType())
                        || !openLink.isAssignableFrom(converter.getReturnType())) continue;
                getter.setAccessible(true);
                Object payload = getter.invoke(response);
                if (payload == null) continue;
                converter.setAccessible(true);
                Object result = converter.invoke(companion, payload);
                if (result != null) return result;
            }
        }
        throw new NoSuchMethodException("open-link conversion signature was not found");
    }

    static Object findRoom(long linkId) throws Exception {
        Object repository = KakaoSignatureResolver.objectFor("room-manager");
        for (Method method : repository.getClass().getDeclaredMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            if (Modifier.isStatic(method.getModifiers())
                    || parameters.length != 1 || parameters[0] != long.class
                    || !List.class.isAssignableFrom(method.getReturnType())) continue;
            try {
                method.setAccessible(true);
                Object result = method.invoke(repository, linkId);
                if (!(result instanceof Iterable)) continue;
                for (Object room : (Iterable<?>) result) {
                    if (room != null && KakaoReflection.hasLongValue(room, linkId)) return room;
                }
            } catch (Throwable ignored) {
            }
        }
        return null;
    }

    static Long linkId(Object openLink) throws Exception {
        if (openLink == null
                || !KakaoSignatureResolver.classFor("open-link").isInstance(openLink)) {
            throw new IllegalArgumentException("invalid OpenLink object");
        }
        Object manager = KakaoSignatureResolver.objectFor("open-link-manager");
        KakaoSignatureResolver.invokeOperation(
                "open-link-cache", manager, new Object[] {openLink});
        String expectedUrl = objectUrl(openLink);
        for (KakaoReflection.LongValue candidate : KakaoReflection.longValues(openLink)) {
            if (candidate.value <= 0) continue;
            Object cached = KakaoSignatureResolver.invokeOperation(
                    "cached-open-link", manager, new Object[] {candidate.value});
            if (cached == openLink || (cached != null && expectedUrl != null
                    && expectedUrl.equals(objectUrl(cached)))) return candidate.value;
        }
        throw new NoSuchMethodException("OpenLink linkId signature was not found");
    }

    static boolean isOpenProfile(Object openLink) throws Exception {
        if (openLink == null
                || !KakaoSignatureResolver.classFor("open-link").isInstance(openLink)) {
            return false;
        }
        for (Method getter : KakaoReflection.allMethods(openLink.getClass())) {
            if (Modifier.isStatic(getter.getModifiers())
                    || getter.getParameterTypes().length != 0
                    || getter.getReturnType().isPrimitive()) continue;
            String type = getter.getReturnType().getName();
            if (!"com.kakao.talk.openlink.model.openprofile.OpenLinkOpenProfile".equals(type)
                    && !"com.kakao.talk.openlink.model.openprofile.OpenCard".equals(type)) {
                continue;
            }
            getter.setAccessible(true);
            Object value = getter.invoke(openLink);
            if (value == null) continue;
            if (type.endsWith("OpenLinkOpenProfile")) return true;
            for (Method flag : KakaoReflection.allMethods(value.getClass())) {
                if (!Modifier.isStatic(flag.getModifiers())
                        && flag.getParameterTypes().length == 0
                        && flag.getReturnType() == boolean.class) {
                    flag.setAccessible(true);
                    if (Boolean.TRUE.equals(flag.invoke(value))) return true;
                }
            }
        }
        return false;
    }

    private static String objectUrl(Object target) {
        if (target == null) return null;
        for (Method method : KakaoReflection.allMethods(target.getClass())) {
            if (Modifier.isStatic(method.getModifiers())
                    || method.getParameterTypes().length != 0
                    || method.getReturnType() != String.class) continue;
            try {
                method.setAccessible(true);
                String value = (String) method.invoke(target);
                if (value != null && value.startsWith("https://open.kakao.com/")) return value;
            } catch (Throwable ignored) {
            }
        }
        return null;
    }

    private static String findOpenProfileUrl(Object target, int depth, Set<Object> visited) {
        if (target == null || depth < 0 || !visited.add(target)) return null;
        if (target instanceof String) {
            String value = (String) target;
            return isOpenProfileUrl(value) ? value : null;
        }
        for (Method method : KakaoReflection.allMethods(target.getClass())) {
            if (Modifier.isStatic(method.getModifiers())
                    || method.getParameterTypes().length != 0
                    || method.getReturnType() == void.class
                    || method.getReturnType().isPrimitive()
                    || method.getDeclaringClass() == Object.class) continue;
            try {
                method.setAccessible(true);
                String value = findOpenProfileUrl(
                        method.invoke(target), depth - 1, visited);
                if (value != null) return value;
            } catch (Throwable ignored) {
            }
        }
        return null;
    }

    private static boolean isOpenProfileUrl(String value) {
        if (value == null) return false;
        String[] prefixes = {"https://open.kakao.com/o/", "https://open.kakao.com/me/"};
        for (String prefix : prefixes) {
            if (!value.startsWith(prefix) || value.length() == prefix.length()) continue;
            for (int index = prefix.length(); index < value.length(); index++) {
                char character = value.charAt(index);
                if (!Character.isLetterOrDigit(character)
                        && character != '_' && character != '-') return false;
            }
            return true;
        }
        return false;
    }
}
