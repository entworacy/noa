package dev.noa.kakao;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.lang.reflect.Proxy;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

/** Resolves KakaoTalk action targets from DEX and reflection signatures. */
public final class KakaoSignatureResolver {
    private static final String ROOM_MANAGER_SOURCE = "ChatRoomListManager.kt";
    private static final String MEMBER_REPOSITORY_SOURCE = "OpenChatMemberRepository.kt";
    private static final String OPEN_LINK_MANAGER_SOURCE = "OlkManager.kt";

    private static final Set<String> TARGET_SOURCES = new HashSet<>(Arrays.asList(
            ROOM_MANAGER_SOURCE,
            "ChatRoomApiHelper.kt",
            MEMBER_REPOSITORY_SOURCE,
            OPEN_LINK_MANAGER_SOURCE,
            "OlkOpenProfileRepository.kt",
            "ConnectionOpenLinkJoin.kt",
            "OpenLinkTypes.kt",
            "ChatSendingLogManager.kt",
            "ChatSendingLogRequest.kt",
            "OpenLink.kt",
            "LocoClient.kt",
            "LocoState.kt",
            "Loco.kt"));

    private static volatile State state;
    private static volatile RoomBinding roomBinding;
    private static volatile KickBinding kickBinding;
    private static volatile FlowBinding locoBinding;
    private static final Map<String, Method> operationCache = new ConcurrentHashMap<>();

    private KakaoSignatureResolver() {}

    /** Returns a room while discovering and caching the repository relationship. */
    public static Object findRoom(long roomId) throws Exception {
        RoomBinding cached = roomBinding;
        if (cached != null) {
            Object room = cached.lookup.invoke(cached.repository, roomId);
            if (room != null && hasLongValue(room, roomId)) {
                return room;
            }
        }
        synchronized (KakaoSignatureResolver.class) {
            cached = discoverRoom(roomId);
            roomBinding = cached;
        }
        Object room = cached.lookup.invoke(cached.repository, roomId);
        if (room == null || !hasLongValue(room, roomId)) {
            throw new IllegalStateException("resolved chat room lookup failed for " + roomId);
        }
        return room;
    }

    /** Performs an open-chat kick using only validated runtime relationships. */
    public static void kick(long roomId, long userId, long commandId) throws Exception {
        Object room = findRoom(roomId);
        KickBinding cached = kickBinding;
        Object member = cached == null ? null : cached.member.invoke(cached.members, userId,
                cached.roomLink.invoke(room));
        if (member == null || !hasLongValue(member, userId)) {
            synchronized (KakaoSignatureResolver.class) {
                cached = discoverKick(room, roomId, userId);
                kickBinding = cached;
            }
            member = cached.member.invoke(cached.members, userId, cached.roomLink.invoke(room));
        }
        if (member == null || !hasLongValue(member, userId)) {
            throw new IllegalStateException("open chat member not found: " + userId);
        }

        Object listener = Proxy.newProxyInstance(
                appState().loader,
                new Class<?>[] {cached.listener},
                new CompletionHandler(commandId, cached.listener));
        cached.kick.invoke(cached.foreground, room, member, false, false, listener);
    }

    /** Resolves an open-profile URL through cache and repository object signatures. */
    public static String openProfileUrl(long linkId) throws Exception {
        Object manager = objectFor("open-link-manager");
        Object cached = invokeOperation("cached-open-link", manager, new Object[] {linkId});
        String url = findOpenProfileUrl(cached, 1,
                Collections.newSetFromMap(new IdentityHashMap<Object, Boolean>()));
        if (url != null) return url;

        Object repository = objectFor("open-profile-repository");
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

    /** Converts a connection response to the stable OpenLink model by type relationships. */
    public static Object convertOpenLink(Object response) throws Exception {
        if (response == null) {
            throw new NullPointerException("open-link connection response is null");
        }
        Object companion = objectFor("open-link");
        Class<?> openLink = classFor("open-link");
        for (Method getter : allMethods(response.getClass())) {
            if (Modifier.isStatic(getter.getModifiers())
                    || getter.getParameterTypes().length != 0
                    || getter.getReturnType().isPrimitive()
                    || getter.getReturnType() == String.class) {
                continue;
            }
            for (Method converter : companion.getClass().getDeclaredMethods()) {
                Class<?>[] parameters = converter.getParameterTypes();
                if (Modifier.isStatic(converter.getModifiers())
                        || parameters.length != 1
                        || !parameters[0].isAssignableFrom(getter.getReturnType())
                        || !openLink.isAssignableFrom(converter.getReturnType())) {
                    continue;
                }
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

    /** Finds a room belonging to an open-link ID across renamed repository methods. */
    public static Object findOpenChatRoom(long linkId) throws Exception {
        Object repository = objectFor("room-manager");
        for (Method method : repository.getClass().getDeclaredMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            if (Modifier.isStatic(method.getModifiers())
                    || parameters.length != 1 || parameters[0] != long.class
                    || !List.class.isAssignableFrom(method.getReturnType())) {
                continue;
            }
            try {
                method.setAccessible(true);
                Object result = method.invoke(repository, linkId);
                if (!(result instanceof Iterable)) continue;
                for (Object room : (Iterable<?>) result) {
                    if (room != null && hasLongValue(room, linkId)) return room;
                }
            } catch (Throwable ignored) {
            }
        }
        return null;
    }

    public static boolean hasLongIdentity(Object target, long value) {
        return target != null && hasLongValue(target, value);
    }

    public static boolean locoConnected() throws Exception {
        FlowBinding cached = locoBinding;
        if (cached == null) {
            synchronized (KakaoSignatureResolver.class) {
                cached = locoBinding;
                if (cached == null) {
                    cached = discoverLocoFlow();
                    locoBinding = cached;
                }
            }
        }
        Object flow = cached.accessor.invoke(cached.singleton);
        Object value = cached.value.invoke(flow);
        return value != null && "connected".equalsIgnoreCase(String.valueOf(value));
    }

    private static FlowBinding discoverLocoFlow() throws Exception {
        List<Class<?>> stateTypes = sourceClasses("LocoState.kt");
        for (Class<?> type : sourceClasses("Loco.kt")) {
            if (type.getName().indexOf('$') >= 0 || type.isInterface()) continue;
            for (Object singleton : singletonObjects(type)) {
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
                        if (current != null && matchesAnyType(current, stateTypes)) {
                            return new FlowBinding(singleton, accessor, value);
                        }
                    } catch (Throwable ignored) {
                    }
                }
            }
        }
        throw new NoSuchMethodException("LOCO state-flow signature was not found");
    }

    private static boolean matchesAnyType(Object value, List<Class<?>> types) {
        for (Class<?> type : types) {
            if (type.isInstance(value)) return true;
        }
        return false;
    }

    /** Resolves OpenLink.linkId by round-tripping candidate IDs through OlkManager. */
    public static Long openLinkId(Object openLink) throws Exception {
        if (openLink == null || !classFor("open-link").isInstance(openLink)) {
            throw new IllegalArgumentException("invalid OpenLink object");
        }
        Object manager = objectFor("open-link-manager");
        invokeOperation("open-link-cache", manager, new Object[] {openLink});
        String expectedUrl = objectUrl(openLink);
        for (LongMethodValue candidate : longValues(openLink)) {
            if (candidate.value <= 0) continue;
            Object cached = invokeOperation(
                    "cached-open-link", manager, new Object[] {candidate.value});
            if (cached == openLink
                    || (cached != null && expectedUrl != null
                    && expectedUrl.equals(objectUrl(cached)))) {
                return candidate.value;
            }
        }
        throw new NoSuchMethodException("OpenLink linkId signature was not found");
    }

    public static boolean isOpenProfile(Object openLink) throws Exception {
        if (openLink == null || !classFor("open-link").isInstance(openLink)) return false;
        for (Method getter : allMethods(openLink.getClass())) {
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
            for (Method flag : allMethods(value.getClass())) {
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

    /** Dispatches resend with a listener proxy matching the current KakaoTalk ABI. */
    public static void resend(Object room, Object entry, long commandId) throws Exception {
        if (room == null || entry == null) {
            throw new IllegalArgumentException("resend room and entry are required");
        }
        Object manager = objectFor("sending-log-manager");
        invokeOperation("prepare-resend", manager, new Object[] {entry});
        Object companion = objectFor("sending-log-request");
        Class<?> modeClass = classFor("sending-log-mode");
        @SuppressWarnings({"unchecked", "rawtypes"})
        Object mode = Enum.valueOf((Class) modeClass, "Resend");

        Method send = null;
        for (Method method : companion.getClass().getDeclaredMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            if (Modifier.isStatic(method.getModifiers())
                    || method.getReturnType() != void.class
                    || parameters.length != 5
                    || !parameters[0].isInstance(room)
                    || !parameters[1].isInstance(entry)
                    || parameters[2] != modeClass
                    || !parameters[3].isInterface()
                    || parameters[4] != boolean.class) {
                continue;
            }
            if (send != null) {
                throw new NoSuchMethodException("ambiguous resend request signature");
            }
            send = method;
        }
        if (send == null) {
            throw new NoSuchMethodException("resend request signature was not found");
        }
        Class<?> listenerType = send.getParameterTypes()[3];
        Object listener = Proxy.newProxyInstance(
                appState().loader,
                new Class<?>[] {listenerType},
                new SendCompletionHandler(commandId));
        send.setAccessible(true);
        send.invoke(companion, room, entry, mode, listener, false);
    }

    private static String objectUrl(Object target) {
        if (target == null) return null;
        for (Method method : allMethods(target.getClass())) {
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
        for (Method method : allMethods(target.getClass())) {
            if (Modifier.isStatic(method.getModifiers())
                    || method.getParameterTypes().length != 0
                    || method.getReturnType() == void.class
                    || method.getReturnType().isPrimitive()
                    || method.getDeclaringClass() == Object.class) {
                continue;
            }
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

    /** Diagnostic text used by startup logs and compatibility reports. */
    public static String describe() throws Exception {
        State value = appState();
        int candidates = 0;
        for (List<String> names : value.classesBySource.values()) {
            candidates += names.size();
        }
        return "source-signatures=" + value.classesBySource.size()
                + ", candidates=" + candidates;
    }

    /**
     * Resolves every startup-safe semantic binding without invoking a KakaoTalk action.
     * This is intentionally driven only by DEX source and call-graph signatures.
     */
    public static String verifySignatures() throws Exception {
        String[] roles = {
            "room-manager",
            "room-api",
            "member-repository",
            "open-link-manager",
            "open-profile-repository",
            "open-link-connection",
            "open-link",
            "open-chat-kakao-profile",
            "open-chat-open-profile",
            "open-chat-profile-use-type",
            "sending-log-manager",
            "sending-log-request",
            "sending-log-mode",
        };
        List<String> resolved = new ArrayList<>();
        for (String role : roles) {
            resolved.add(role + "=" + classFor(role).getName());
        }
        resolved.add(verifyCallGraphOperation("chat-on-room", "room-api"));
        resolved.add(verifyCallGraphOperation("join-link", "room-api"));
        resolved.add(verifyCallGraphOperation("prepare-resend", "sending-log-manager"));
        FlowBinding flow = discoverLocoFlow();
        locoBinding = flow;
        resolved.add("loco-state=" + flow.accessor.getDeclaringClass().getName()
                + "." + flow.accessor.getName());
        return String.join(", ", resolved);
    }

    /** Returns classes compiled from a source file, as recorded in the APK DEX. */
    public static String[] sourceClassNames(String source) throws Exception {
        List<String> names = appState().classesBySource.get(source);
        if (names == null || names.isEmpty()) {
            throw new ClassNotFoundException("DEX source signature was not found: " + source);
        }
        return names.toArray(new String[0]);
    }

    private static String verifyCallGraphOperation(String operation, String targetRole)
            throws Exception {
        Object target = objectFor(targetRole);
        List<MethodKey> keys = appState().operations.get(operation);
        if (keys == null || keys.isEmpty()) {
            throw new NoSuchMethodException("DEX call-graph signature was not found: " + operation);
        }
        Set<String> matchedNames = new LinkedHashSet<>();
        int overloads = 0;
        for (MethodKey key : keys) {
            if (!key.owner.equals(target.getClass().getName())) continue;
            for (Method method : target.getClass().getDeclaredMethods()) {
                if (!Modifier.isStatic(method.getModifiers()) && method.getName().equals(key.name)) {
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

    /** Resolves a semantic class role without relying on an obfuscated class name. */
    public static Class<?> classFor(String role) throws Exception {
        if ("room-manager".equals(role)) {
            return largestOuter(ROOM_MANAGER_SOURCE);
        }
        if ("room-api".equals(role)) {
            return largestOuter("ChatRoomApiHelper.kt");
        }
        if ("member-repository".equals(role)) {
            return largestOuter(MEMBER_REPOSITORY_SOURCE);
        }
        if ("open-link-manager".equals(role)) {
            return largestOuter(OPEN_LINK_MANAGER_SOURCE);
        }
        if ("open-profile-repository".equals(role)) {
            return largestOuter("OlkOpenProfileRepository.kt");
        }
        if ("open-link-connection".equals(role)) {
            for (Class<?> type : sourceClasses("ConnectionOpenLinkJoin.kt")) {
                for (java.lang.reflect.Constructor<?> constructor : type.getDeclaredConstructors()) {
                    Class<?>[] parameters = constructor.getParameterTypes();
                    if (parameters.length == 1
                            && "android.content.Intent".equals(parameters[0].getName())) {
                        return type;
                    }
                }
            }
        }
        if ("open-link".equals(role)) {
            for (Class<?> type : sourceClasses("OpenLink.kt")) {
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
            for (Class<?> type : sourceClasses("OpenLinkTypes.kt")) {
                for (Field field : type.getDeclaredFields()) {
                    if (!Modifier.isStatic(field.getModifiers())) continue;
                    if (hasFactory(field.getType(), long.class, true)) return type;
                }
            }
        }
        if ("open-chat-profile-use-type".equals(role)) {
            Class<?> profile = classFor("open-chat-open-profile");
            for (Class<?> nested : sourceClasses("OpenLinkTypes.kt")) {
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
            for (Class<?> type : sourceClasses("ChatSendingLogRequest.kt")) {
                if (!type.isEnum()) continue;
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

    /** Returns the singleton or Kotlin companion associated with a semantic role. */
    public static Object objectFor(String role) throws Exception {
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
        List<Object> singletons = singletonObjects(type);
        if (!singletons.isEmpty()) {
            return singletons.get(0);
        }
        throw new NoSuchFieldException("singleton signature was not found for role: " + role);
    }

    /** Returns a static value by its stable Java supertype instead of a field name. */
    public static Object staticValueFor(String role, String typeName) throws Exception {
        Class<?> owner = classFor(role);
        Class<?> expected = Class.forName(typeName, false, appState().loader);
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

    /** Invokes a method selected by a protocol/call-graph signature. */
    public static Object invokeOperation(String operation, Object target, Object[] arguments)
            throws Exception {
        if (target == null) {
            throw new NullPointerException(operation + " target is null");
        }
        Object[] actual = arguments == null ? new Object[0] : arguments;
        String cacheKey = operation + "@" + target.getClass().getName();
        Method method = operationCache.get(cacheKey);
        if (method == null || !argumentsMatch(method, actual)) {
            method = resolveOperation(operation, target.getClass(), actual);
            method.setAccessible(true);
            operationCache.put(cacheKey, method);
        }
        return method.invoke(target, actual);
    }

    private static Method resolveOperation(
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
            if (matched != null) return matched;
        }
        if ("load-sending-log".equals(operation)) {
            Method matched = null;
            for (Method method : target.getDeclaredMethods()) {
                Class<?>[] parameters = method.getParameterTypes();
                if (Modifier.isStatic(method.getModifiers())
                        || parameters.length != 1
                        || !"kotlin.coroutines.Continuation".equals(parameters[0].getName())
                        || method.getReturnType() != Object.class) continue;
                if (matched != null) {
                    throw new NoSuchMethodException("ambiguous sending-log load signature");
                }
                matched = method;
            }
            if (matched != null) return matched;
        }
        Class<?> openLink = null;
        if (operation.startsWith("open-link-")
                || "cached-open-link".equals(operation)
                || "apply-open-profile".equals(operation)
                || "create-open-chat".equals(operation)) {
            openLink = classFor("open-link");
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
            if (!match || !argumentsMatch(method, arguments)) continue;
            if (shaped != null) {
                throw new NoSuchMethodException("ambiguous " + operation + " signature");
            }
            shaped = method;
        }
        if (shaped != null) return shaped;
        List<MethodKey> keys = appState().operations.get(operation);
        if (keys != null) {
            for (MethodKey key : keys) {
                if (!key.owner.equals(target.getName())) continue;
                for (Method method : target.getDeclaredMethods()) {
                    if (method.getName().equals(key.name) && argumentsMatch(method, arguments)) {
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
                if (parameters[index].isPrimitive()) return false;
                continue;
            }
            Class<?> expected = boxed(parameters[index]);
            if (!expected.isInstance(argument)) return false;
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
                if (value == null) continue;
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
        for (Class<?> type : sourceClasses(source)) {
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
        for (Class<?> type : sourceClasses("OpenLinkTypes.kt")) {
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

    private static RoomBinding discoverRoom(long roomId) throws Exception {
        List<String> failures = new ArrayList<>();
        for (Class<?> type : sourceClasses(ROOM_MANAGER_SOURCE)) {
            if (type.isInterface() || type.isEnum() || type.getDeclaredMethods().length < 20) {
                continue;
            }
            for (Object repository : singletonObjects(type)) {
                for (Method method : type.getDeclaredMethods()) {
                    Class<?>[] parameters = method.getParameterTypes();
                    if (Modifier.isStatic(method.getModifiers())
                            || parameters.length != 1
                            || parameters[0] != long.class
                            || method.getReturnType().isPrimitive()
                            || method.getReturnType() == String.class
                            || method.getReturnType().isArray()) {
                        continue;
                    }
                    try {
                        method.setAccessible(true);
                        Object room = method.invoke(repository, roomId);
                        if (room != null && hasLongValue(room, roomId)) {
                            return new RoomBinding(repository, method);
                        }
                    } catch (Throwable error) {
                        remember(failures, type.getName() + "." + method.getName(), error);
                    }
                }
            }
        }
        throw new NoSuchMethodException("chat-room signature was not found: "
                + lastFailure(failures));
    }

    private static KickBinding discoverKick(Object room, long roomId, long userId)
            throws Exception {
        List<LongMethodValue> roomValues = longValues(room);
        List<String> failures = new ArrayList<>();
        MemberMatch memberMatch = null;

        for (Class<?> type : sourceClasses(MEMBER_REPOSITORY_SOURCE)) {
            for (Object repository : singletonObjects(type)) {
                for (Method method : type.getDeclaredMethods()) {
                    Class<?>[] parameters = method.getParameterTypes();
                    if (Modifier.isStatic(method.getModifiers())
                            || parameters.length != 2
                            || parameters[0] != long.class
                            || parameters[1] != long.class
                            || method.getReturnType().isPrimitive()
                            || method.getReturnType() == Object.class) {
                        continue;
                    }
                    method.setAccessible(true);
                    for (LongMethodValue value : roomValues) {
                        if (value.value <= 0 || value.value == roomId) {
                            continue;
                        }
                        try {
                            Object member = method.invoke(repository, userId, value.value);
                            if (member != null && hasLongValue(member, userId)) {
                                memberMatch = new MemberMatch(
                                        repository, method, value.method, member);
                                break;
                            }
                        } catch (Throwable error) {
                            remember(failures, type.getName() + "." + method.getName(), error);
                        }
                    }
                    if (memberMatch != null) {
                        break;
                    }
                }
                if (memberMatch != null) {
                    break;
                }
            }
            if (memberMatch != null) {
                break;
            }
        }
        if (memberMatch == null) {
            throw new NoSuchMethodException("open-chat member signature was not found: "
                    + lastFailure(failures));
        }

        for (Class<?> type : sourceClasses(OPEN_LINK_MANAGER_SOURCE)) {
            for (Object manager : singletonObjects(type)) {
                for (Method accessor : type.getDeclaredMethods()) {
                    if (Modifier.isStatic(accessor.getModifiers())
                            || accessor.getParameterTypes().length != 0
                            || accessor.getReturnType().isPrimitive()
                            || accessor.getReturnType() == String.class) {
                        continue;
                    }
                    Method kick = kickMethod(
                            accessor.getReturnType(), room.getClass(), memberMatch.value.getClass());
                    if (kick == null) {
                        continue;
                    }
                    try {
                        accessor.setAccessible(true);
                        Object foreground = accessor.invoke(manager);
                        if (foreground == null) {
                            continue;
                        }
                        Class<?> listener = kick.getParameterTypes()[4];
                        if (!listener.isInterface()) {
                            continue;
                        }
                        memberMatch.method.setAccessible(true);
                        memberMatch.roomLink.setAccessible(true);
                        kick.setAccessible(true);
                        return new KickBinding(
                                memberMatch.repository,
                                memberMatch.method,
                                memberMatch.roomLink,
                                foreground,
                                kick,
                                listener);
                    } catch (Throwable error) {
                        remember(failures, type.getName() + "." + accessor.getName(), error);
                    }
                }
            }
        }
        throw new NoSuchMethodException("open-chat kick signature was not found: "
                + lastFailure(failures));
    }

    private static Method kickMethod(Class<?> foreground, Class<?> room, Class<?> member) {
        Method matched = null;
        for (Method method : allMethods(foreground)) {
            Class<?>[] parameters = method.getParameterTypes();
            if (Modifier.isStatic(method.getModifiers())
                    || method.getReturnType() != void.class
                    || parameters.length != 5
                    || !parameters[0].isAssignableFrom(room)
                    || !parameters[1].isAssignableFrom(member)
                    || parameters[2] != boolean.class
                    || parameters[3] != boolean.class
                    || !parameters[4].isInterface()) {
                continue;
            }
            if (matched != null) {
                return null;
            }
            matched = method;
        }
        return matched;
    }

    private static boolean hasLongValue(Object target, long expected) {
        try {
            for (LongMethodValue value : longValues(target)) {
                if (value.value == expected) {
                    return true;
                }
            }
        } catch (Throwable ignored) {
        }
        return false;
    }

    private static List<LongMethodValue> longValues(Object target) {
        List<LongMethodValue> values = new ArrayList<>();
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
                    values.add(new LongMethodValue(method, ((Number) result).longValue()));
                }
            } catch (Throwable ignored) {
            }
        }
        return values;
    }

    private static List<Method> allMethods(Class<?> type) {
        Map<String, Method> methods = new LinkedHashMap<>();
        for (Class<?> current = type; current != null; current = current.getSuperclass()) {
            for (Method method : current.getDeclaredMethods()) {
                String key = method.getName() + Arrays.toString(method.getParameterTypes());
                if (!methods.containsKey(key)) {
                    methods.put(key, method);
                }
            }
        }
        for (Class<?> contract : type.getInterfaces()) {
            for (Method method : contract.getMethods()) {
                String key = method.getName() + Arrays.toString(method.getParameterTypes());
                if (!methods.containsKey(key)) {
                    methods.put(key, method);
                }
            }
        }
        return new ArrayList<>(methods.values());
    }

    private static List<Object> singletonObjects(Class<?> type) {
        List<Object> values = new ArrayList<>();
        for (Field field : type.getDeclaredFields()) {
            if (!Modifier.isStatic(field.getModifiers())) {
                continue;
            }
            Class<?> fieldType = field.getType();
            if (!type.isAssignableFrom(fieldType)
                    && !fieldType.getName().startsWith(type.getName() + "$")) {
                continue;
            }
            try {
                field.setAccessible(true);
                Object value = field.get(null);
                if (value == null) {
                    continue;
                }
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
                    if (singleton != null) {
                        values.add(singleton);
                    }
                }
            } catch (Throwable ignored) {
            }
        }
        return values;
    }

    private static List<Class<?>> sourceClasses(String source) throws Exception {
        State value = appState();
        List<String> names = value.classesBySource.get(source);
        if (names == null || names.isEmpty()) {
            throw new ClassNotFoundException("DEX source signature was not found: " + source);
        }
        List<Class<?>> classes = new ArrayList<>();
        for (String name : names) {
            try {
                classes.add(Class.forName(name, false, value.loader));
            } catch (Throwable ignored) {
            }
        }
        return classes;
    }

    private static State appState() throws Exception {
        State cached = state;
        if (cached != null) {
            return cached;
        }
        synchronized (KakaoSignatureResolver.class) {
            cached = state;
            if (cached == null) {
                Object application = currentApplication();
                Class<?> context = Class.forName("android.content.Context");
                ClassLoader loader = (ClassLoader) context.getMethod("getClassLoader")
                        .invoke(application);
                DexIndex index = sourceIndex(application);
                cached = new State(loader, index.classesBySource, index.operations);
                state = cached;
            }
        }
        return cached;
    }

    private static Object currentApplication() throws Exception {
        Class<?> activityThread = Class.forName("android.app.ActivityThread");
        Object application = activityThread.getMethod("currentApplication").invoke(null);
        if (application == null) {
            throw new IllegalStateException("Android application is not ready");
        }
        return application;
    }

    private static DexIndex sourceIndex(Object application) throws Exception {
        Object info = Class.forName("android.content.Context")
                .getMethod("getApplicationInfo")
                .invoke(application);
        List<String> paths = new ArrayList<>();
        paths.add((String) publicField(info, "sourceDir"));
        Object splits = publicField(info, "splitSourceDirs");
        if (splits instanceof String[]) {
            Collections.addAll(paths, (String[]) splits);
        }
        Map<String, LinkedHashSet<String>> collected = new LinkedHashMap<>();
        Map<String, LinkedHashSet<MethodKey>> operations = new LinkedHashMap<>();
        for (String path : paths) {
            if (path == null || path.isEmpty()) {
                continue;
            }
            try (ZipFile archive = new ZipFile(path)) {
                List<? extends ZipEntry> entries = Collections.list(archive.entries());
                entries.sort(Comparator.comparing(ZipEntry::getName));
                for (ZipEntry entry : entries) {
                    String name = entry.getName();
                    if (!name.startsWith("classes") || !name.endsWith(".dex")) {
                        continue;
                    }
                    try (InputStream input = archive.getInputStream(entry)) {
                        byte[] dex = readAll(input);
                        indexDex(dex, collected);
                        indexOperations(dex, operations);
                    }
                }
            } catch (Throwable ignored) {
                // Resource-only or vendor splits may not be readable as APK archives.
            }
        }
        Map<String, List<String>> result = new LinkedHashMap<>();
        for (Map.Entry<String, LinkedHashSet<String>> entry : collected.entrySet()) {
            result.put(entry.getKey(), new ArrayList<>(entry.getValue()));
        }
        Map<String, List<MethodKey>> resolvedOperations = new LinkedHashMap<>();
        for (Map.Entry<String, LinkedHashSet<MethodKey>> entry : operations.entrySet()) {
            resolvedOperations.put(entry.getKey(), new ArrayList<>(entry.getValue()));
        }
        return new DexIndex(result, resolvedOperations);
    }

    private static void indexDex(
            byte[] dex, Map<String, LinkedHashSet<String>> classesBySource) {
        if (dex.length < 112
                || dex[0] != 'd' || dex[1] != 'e' || dex[2] != 'x' || dex[3] != '\n') {
            return;
        }
        int stringCount = littleInt(dex, 0x38);
        int stringOffset = littleInt(dex, 0x3c);
        int typeCount = littleInt(dex, 0x40);
        int typeOffset = littleInt(dex, 0x44);
        int classCount = littleInt(dex, 0x60);
        int classOffset = littleInt(dex, 0x64);
        if (!tableFits(dex, stringOffset, stringCount, 4)
                || !tableFits(dex, typeOffset, typeCount, 4)
                || !tableFits(dex, classOffset, classCount, 32)) {
            return;
        }
        Map<Integer, String> strings = new HashMap<>();
        for (int index = 0; index < classCount; index++) {
            int item = classOffset + index * 32;
            int classIndex = littleInt(dex, item);
            int sourceIndex = littleInt(dex, item + 16);
            if (classIndex < 0 || classIndex >= typeCount
                    || sourceIndex < 0 || sourceIndex >= stringCount) {
                continue;
            }
            String source = dexString(dex, stringOffset, sourceIndex, strings);
            if (!TARGET_SOURCES.contains(source)) {
                continue;
            }
            int descriptorIndex = littleInt(dex, typeOffset + classIndex * 4);
            if (descriptorIndex < 0 || descriptorIndex >= stringCount) {
                continue;
            }
            String descriptor = dexString(dex, stringOffset, descriptorIndex, strings);
            if (descriptor.length() < 3 || descriptor.charAt(0) != 'L'
                    || descriptor.charAt(descriptor.length() - 1) != ';') {
                continue;
            }
            String className = descriptor.substring(1, descriptor.length() - 1)
                    .replace('/', '.');
            classesBySource.computeIfAbsent(source, ignored -> new LinkedHashSet<>())
                    .add(className);
        }
    }

    private static void indexOperations(
            byte[] dex, Map<String, LinkedHashSet<MethodKey>> operations) {
        if (dex.length < 112
                || dex[0] != 'd' || dex[1] != 'e' || dex[2] != 'x' || dex[3] != '\n') {
            return;
        }
        int stringCount = littleInt(dex, 0x38);
        int stringOffset = littleInt(dex, 0x3c);
        int typeCount = littleInt(dex, 0x40);
        int typeOffset = littleInt(dex, 0x44);
        int fieldCount = littleInt(dex, 0x50);
        int fieldOffset = littleInt(dex, 0x54);
        int methodCount = littleInt(dex, 0x58);
        int methodOffset = littleInt(dex, 0x5c);
        int classCount = littleInt(dex, 0x60);
        int classOffset = littleInt(dex, 0x64);
        if (!tableFits(dex, stringOffset, stringCount, 4)
                || !tableFits(dex, typeOffset, typeCount, 4)
                || !tableFits(dex, fieldOffset, fieldCount, 8)
                || !tableFits(dex, methodOffset, methodCount, 8)
                || !tableFits(dex, classOffset, classCount, 32)) {
            return;
        }
        Map<Integer, String> strings = new HashMap<>();
        Map<Integer, String> protocolFields = new HashMap<>();
        for (int index = 0; index < fieldCount; index++) {
            int nameIndex = littleInt(dex, fieldOffset + index * 8 + 4);
            if (nameIndex < 0 || nameIndex >= stringCount) continue;
            String name = dexString(dex, stringOffset, nameIndex, strings);
            if ("CHATONROOM".equals(name) || "JOINLINK".equals(name)) {
                protocolFields.put(index, name);
            }
        }
        MethodKey[] methodKeys = new MethodKey[methodCount];
        for (int index = 0; index < methodCount; index++) {
            int item = methodOffset + index * 8;
            int ownerIndex = unsignedShort(dex, item);
            int nameIndex = littleInt(dex, item + 4);
            if (ownerIndex < 0 || ownerIndex >= typeCount
                    || nameIndex < 0 || nameIndex >= stringCount) continue;
            int descriptorIndex = littleInt(dex, typeOffset + ownerIndex * 4);
            String descriptor = dexString(dex, stringOffset, descriptorIndex, strings);
            String name = dexString(dex, stringOffset, nameIndex, strings);
            if (descriptor.length() >= 3 && descriptor.charAt(0) == 'L') {
                methodKeys[index] = new MethodKey(
                        descriptor.substring(1, descriptor.length() - 1).replace('/', '.'), name);
            }
        }

        Map<Integer, MethodCode> helperMethods = new LinkedHashMap<>();
        Map<Integer, MethodCode> sendingMethods = new LinkedHashMap<>();
        for (int index = 0; index < classCount; index++) {
            int item = classOffset + index * 32;
            int sourceIndex = littleInt(dex, item + 16);
            int classData = littleInt(dex, item + 24);
            if (sourceIndex < 0 || sourceIndex >= stringCount || classData <= 0) continue;
            String source = dexString(dex, stringOffset, sourceIndex, strings);
            if ("ChatRoomApiHelper.kt".equals(source)) {
                readMethodCodeItems(dex, classData, helperMethods);
            } else if ("ChatSendingLogManager.kt".equals(source)) {
                readMethodCodeItems(dex, classData, sendingMethods);
            }
        }

        Map<String, LinkedHashSet<Integer>> direct = new LinkedHashMap<>();
        for (Map.Entry<Integer, MethodCode> entry : helperMethods.entrySet()) {
            for (Integer field : entry.getValue().fields) {
                String semantic = protocolFields.get(field);
                if (semantic != null) {
                    direct.computeIfAbsent(semantic, ignored -> new LinkedHashSet<>())
                            .add(entry.getKey());
                }
            }
        }
        addCallers("chat-on-room", direct.get("CHATONROOM"), helperMethods, methodKeys, operations);
        addCallers("join-link", direct.get("JOINLINK"), helperMethods, methodKeys, operations);
        for (Map.Entry<Integer, MethodCode> entry : sendingMethods.entrySet()) {
            MethodKey owner = entry.getKey() < methodKeys.length ? methodKeys[entry.getKey()] : null;
            if (owner == null || owner.owner.indexOf('$') >= 0) continue;
            boolean chatLogCall = false;
            boolean ownBridge = false;
            boolean innerCall = false;
            for (Integer calledIndex : entry.getValue().calls) {
                MethodKey called = calledIndex < methodKeys.length ? methodKeys[calledIndex] : null;
                if (called == null) continue;
                if ("com.kakao.talk.manager.send.sending.ChatSendingLog".equals(called.owner)) {
                    chatLogCall = true;
                } else if (owner.owner.equals(called.owner)) {
                    ownBridge = true;
                } else if (called.owner.startsWith(owner.owner + "$")) {
                    innerCall = true;
                }
            }
            if (chatLogCall && ownBridge && innerCall) {
                operations.computeIfAbsent("prepare-resend", ignored -> new LinkedHashSet<>())
                        .add(owner);
            }
        }
    }

    private static void addCallers(
            String operation,
            Set<Integer> direct,
            Map<Integer, MethodCode> methods,
            MethodKey[] keys,
            Map<String, LinkedHashSet<MethodKey>> operations) {
        if (direct == null || direct.isEmpty()) return;
        LinkedHashSet<MethodKey> callers = new LinkedHashSet<>();
        for (Map.Entry<Integer, MethodCode> entry : methods.entrySet()) {
            if (direct.contains(entry.getKey())) continue;
            for (Integer called : entry.getValue().calls) {
                if (direct.contains(called) && entry.getKey() < keys.length
                        && keys[entry.getKey()] != null) {
                    callers.add(keys[entry.getKey()]);
                }
            }
        }
        if (callers.isEmpty()) {
            for (Integer index : direct) {
                if (index < keys.length && keys[index] != null) callers.add(keys[index]);
            }
        }
        operations.computeIfAbsent(operation, ignored -> new LinkedHashSet<>()).addAll(callers);
    }

    private static void readMethodCodeItems(
            byte[] dex, int classData, Map<Integer, MethodCode> methods) {
        try {
            Cursor cursor = new Cursor(classData);
            int staticFields = readUleb128(dex, cursor);
            int instanceFields = readUleb128(dex, cursor);
            int directMethods = readUleb128(dex, cursor);
            int virtualMethods = readUleb128(dex, cursor);
            for (int index = 0; index < staticFields + instanceFields; index++) {
                readUleb128(dex, cursor);
                readUleb128(dex, cursor);
            }
            readEncodedMethods(dex, cursor, directMethods, methods);
            readEncodedMethods(dex, cursor, virtualMethods, methods);
        } catch (Throwable ignored) {
        }
    }

    private static void readEncodedMethods(
            byte[] dex, Cursor cursor, int count, Map<Integer, MethodCode> methods) {
        int methodIndex = 0;
        for (int index = 0; index < count; index++) {
            methodIndex += readUleb128(dex, cursor);
            readUleb128(dex, cursor);
            int codeOffset = readUleb128(dex, cursor);
            if (codeOffset > 0) {
                methods.put(methodIndex, scanCode(dex, codeOffset));
            }
        }
    }

    private static MethodCode scanCode(byte[] dex, int codeOffset) {
        MethodCode result = new MethodCode();
        if (codeOffset < 0 || codeOffset > dex.length - 16) return result;
        int count = littleInt(dex, codeOffset + 12);
        int start = codeOffset + 16;
        int end = (int) Math.min((long) dex.length, (long) start + (long) count * 2);
        for (int cursor = start; cursor + 3 < end; cursor += 2) {
            int opcode = dex[cursor] & 0xff;
            int reference = unsignedShort(dex, cursor + 2);
            if (opcode == 0x62) {
                result.fields.add(reference);
            } else if ((opcode >= 0x6e && opcode <= 0x72)
                    || (opcode >= 0x74 && opcode <= 0x78)) {
                result.calls.add(reference);
            }
        }
        return result;
    }

    private static int readUleb128(byte[] value, Cursor cursor) {
        int result = 0;
        int shift = 0;
        for (int index = 0; index < 5; index++) {
            int next = value[cursor.value++] & 0xff;
            result |= (next & 0x7f) << shift;
            if ((next & 0x80) == 0) return result;
            shift += 7;
        }
        throw new IllegalArgumentException("invalid ULEB128");
    }

    private static int unsignedShort(byte[] value, int offset) {
        if (offset < 0 || offset > value.length - 2) return -1;
        return (value[offset] & 0xff) | ((value[offset + 1] & 0xff) << 8);
    }

    private static String dexString(
            byte[] dex, int stringOffset, int index, Map<Integer, String> cache) {
        String cached = cache.get(index);
        if (cached != null) {
            return cached;
        }
        int data = littleInt(dex, stringOffset + index * 4);
        if (data < 0 || data >= dex.length) {
            return "";
        }
        int cursor = data;
        while (cursor < dex.length && (dex[cursor++] & 0x80) != 0) {
        }
        int end = cursor;
        while (end < dex.length && dex[end] != 0) {
            end++;
        }
        String value;
        try {
            value = new String(dex, cursor, end - cursor, "UTF-8");
        } catch (Throwable ignored) {
            value = "";
        }
        cache.put(index, value);
        return value;
    }

    private static byte[] readAll(InputStream input) throws Exception {
        ByteArrayOutputStream output = new ByteArrayOutputStream(1024 * 1024);
        byte[] buffer = new byte[64 * 1024];
        int count;
        while ((count = input.read(buffer)) != -1) {
            output.write(buffer, 0, count);
        }
        return output.toByteArray();
    }

    private static int littleInt(byte[] value, int offset) {
        if (offset < 0 || offset > value.length - 4) {
            return -1;
        }
        return (value[offset] & 0xff)
                | ((value[offset + 1] & 0xff) << 8)
                | ((value[offset + 2] & 0xff) << 16)
                | ((value[offset + 3] & 0xff) << 24);
    }

    private static boolean tableFits(byte[] value, int offset, int count, int width) {
        return offset >= 0 && count >= 0
                && (long) offset + (long) count * width <= value.length;
    }

    private static Object publicField(Object target, String name) throws Exception {
        Field field = target.getClass().getField(name);
        return field.get(target);
    }

    private static void remember(List<String> failures, String target, Throwable error) {
        if (failures.size() < 24) {
            failures.add(target + ": " + error);
        }
    }

    private static String lastFailure(List<String> failures) {
        return failures.isEmpty() ? "no candidate matched"
                : failures.get(failures.size() - 1);
    }

    private static final class CompletionHandler implements InvocationHandler {
        private final long commandId;
        private final String successMethod;

        CompletionHandler(long commandId, Class<?> listener) {
            this.commandId = commandId;
            List<Method> callbacks = new ArrayList<>();
            for (Method method : listener.getMethods()) {
                if (method.getDeclaringClass() != Object.class
                        && method.getReturnType() == void.class) {
                    callbacks.add(method);
                }
            }
            callbacks.sort(Comparator.comparing(Method::getName));
            this.successMethod = callbacks.isEmpty()
                    ? "" : callbacks.get(callbacks.size() - 1).getName();
        }

        @Override
        public Object invoke(Object proxy, Method method, Object[] arguments) {
            if (method.getDeclaringClass() == Object.class) {
                if ("toString".equals(method.getName())) {
                    return "NoaCompletionProxy(" + commandId + ")";
                }
                if ("hashCode".equals(method.getName())) {
                    return System.identityHashCode(proxy);
                }
                if ("equals".equals(method.getName())) {
                    return arguments != null && arguments.length == 1 && proxy == arguments[0];
                }
            }
            boolean success = method.getName().equals(successMethod)
                    && (arguments == null || arguments.length == 0);
            String error = success ? null : callbackError(method, arguments);
            Bridge.complete(commandId, success, error);
            return primitiveDefault(method.getReturnType());
        }
    }

    private static final class SendCompletionHandler implements InvocationHandler {
        private final long commandId;

        SendCompletionHandler(long commandId) {
            this.commandId = commandId;
        }

        @Override
        public Object invoke(Object proxy, Method method, Object[] arguments) {
            if (method.getDeclaringClass() == Object.class) {
                if ("toString".equals(method.getName())) {
                    return "NoaSendCompletionProxy(" + commandId + ")";
                }
                if ("hashCode".equals(method.getName())) return System.identityHashCode(proxy);
                if ("equals".equals(method.getName())) {
                    return arguments != null && arguments.length == 1 && proxy == arguments[0];
                }
            }
            Class<?>[] parameters = method.getParameterTypes();
            boolean success = parameters.length == 3 && parameters[1] == long.class;
            Bridge.complete(commandId, success,
                    success ? null : callbackError(method, arguments));
            return primitiveDefault(method.getReturnType());
        }
    }

    private static String callbackError(Method method, Object[] arguments) {
        StringBuilder message = new StringBuilder("KakaoTalk request failed: ")
                .append(method.getName());
        if (arguments != null && arguments.length > 0) {
            message.append(' ');
            for (int index = 0; index < arguments.length; index++) {
                if (index > 0) {
                    message.append(", ");
                }
                message.append(String.valueOf(arguments[index]));
            }
        }
        return message.toString();
    }

    private static Object primitiveDefault(Class<?> type) {
        if (!type.isPrimitive() || type == void.class) return null;
        if (type == boolean.class) return false;
        if (type == char.class) return '\0';
        if (type == byte.class) return (byte) 0;
        if (type == short.class) return (short) 0;
        if (type == int.class) return 0;
        if (type == long.class) return 0L;
        if (type == float.class) return 0F;
        if (type == double.class) return 0D;
        return null;
    }

    private static final class State {
        final ClassLoader loader;
        final Map<String, List<String>> classesBySource;
        final Map<String, List<MethodKey>> operations;

        State(
                ClassLoader loader,
                Map<String, List<String>> classesBySource,
                Map<String, List<MethodKey>> operations) {
            this.loader = loader;
            this.classesBySource = classesBySource;
            this.operations = operations;
        }
    }

    private static final class DexIndex {
        final Map<String, List<String>> classesBySource;
        final Map<String, List<MethodKey>> operations;

        DexIndex(
                Map<String, List<String>> classesBySource,
                Map<String, List<MethodKey>> operations) {
            this.classesBySource = classesBySource;
            this.operations = operations;
        }
    }

    private static final class MethodKey {
        final String owner;
        final String name;

        MethodKey(String owner, String name) {
            this.owner = owner;
            this.name = name;
        }

        @Override
        public boolean equals(Object other) {
            if (!(other instanceof MethodKey)) return false;
            MethodKey value = (MethodKey) other;
            return owner.equals(value.owner) && name.equals(value.name);
        }

        @Override
        public int hashCode() {
            return owner.hashCode() * 31 + name.hashCode();
        }

        @Override
        public String toString() {
            return owner + "." + name;
        }
    }

    private static final class MethodCode {
        final Set<Integer> fields = new LinkedHashSet<>();
        final Set<Integer> calls = new LinkedHashSet<>();
    }

    private static final class Cursor {
        int value;

        Cursor(int value) {
            this.value = value;
        }
    }

    private static final class RoomBinding {
        final Object repository;
        final Method lookup;

        RoomBinding(Object repository, Method lookup) {
            this.repository = repository;
            this.lookup = lookup;
        }
    }

    private static final class FlowBinding {
        final Object singleton;
        final Method accessor;
        final Method value;

        FlowBinding(Object singleton, Method accessor, Method value) {
            this.singleton = singleton;
            this.accessor = accessor;
            this.value = value;
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

        KickBinding(
                Object members,
                Method member,
                Method roomLink,
                Object foreground,
                Method kick,
                Class<?> listener) {
            this.members = members;
            this.member = member;
            this.roomLink = roomLink;
            this.foreground = foreground;
            this.kick = kick;
            this.listener = listener;
        }
    }

    private static final class LongMethodValue {
        final Method method;
        final long value;

        LongMethodValue(Method method, long value) {
            this.method = method;
            this.value = value;
        }
    }
}
