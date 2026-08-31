package dev.noa.kakao;

import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.lang.reflect.Proxy;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicBoolean;

/** Version-tolerant reflection adapter for KakaoTalk's fused vox_main feature. */
public final class VoxController {
    private static final String FACADE =
            "com.kakao.talk.module.vox.contract.VoxModuleFacade";
    private static final String FACADE_FACTORY =
            "com.kakao.talk.vox.VoxModuleFacadeFactory";
    private static final String MAKING_INFO =
            "com.kakao.talk.module.vox.domain.model.cecall.CecallMakingInfo";
    private static final String OUTGOING_AUTO =
            "com.kakao.talk.module.vox.domain.model.cecall.CecallOutgoingTrigger$Auto";
    private static final String VOICEROOM_ACTIVITY =
            "com.kakao.talk.vox.ui.voiceroom.VoiceroomActivity";

    private static volatile Object facade;

    private VoxController() {}

    public static void startCecall(
            long commandId,
            long chatId,
            long callerId,
            String peerIds,
            boolean openChat,
            boolean teamChat,
            boolean groupChat) {
        try {
            Object context = application();
            ClassLoader loader = context.getClass().getClassLoader();
            Class<?> makingType = load(loader, MAKING_INFO);
            Constructor<?> constructor = makingConstructor(makingType);
            Class<?> callType = constructor.getParameterTypes()[4];
            Object voiceTalk = enumConstant(callType, "VOICETALK");
            Object makingInfo = constructor.newInstance(
                    chatId,
                    0L,
                    callerId,
                    parsePeers(peerIds),
                    voiceTalk,
                    openChat,
                    teamChat,
                    groupChat,
                    false);
            Object trigger = singleton(load(loader, OUTGOING_AUTO));
            Object vox = facade(context);
            Method start = exactMethod(vox.getClass(), "ensurePermissionsAndMakeCall", 3);
            start.invoke(vox, context, makingInfo, trigger);
            Bridge.complete(commandId, true, null);
        } catch (Throwable error) {
            completeFailure(commandId, "start VoiceTalk", error);
        }
    }

    public static void createVoiceroom(long commandId, long chatId, String title) {
        try {
            Object context = application();
            requireMicrophonePermission(context);
            Object manager = invokeNoArgs(facade(context), "getVoiceroomManager");
            Completion completion = new Completion(commandId, context, true);
            Method method = exactMethod(manager.getClass(), "makeVoiceroom", 5);
            Class<?>[] types = method.getParameterTypes();
            method.invoke(
                    manager,
                    context,
                    chatId,
                    title,
                    completion.callback(types[3], true),
                    completion.callback(types[4], false));
        } catch (Throwable error) {
            completeFailure(commandId, "create voice room", error);
        }
    }

    public static void joinVoiceroom(
            long commandId,
            long chatId,
            long callId,
            String hostV4,
            String hostV6,
            int port) {
        try {
            Object context = application();
            requireMicrophonePermission(context);
            Object manager = invokeNoArgs(facade(context), "getVoiceroomManager");
            Method method = exactMethod(manager.getClass(), "joinVoiceroom", 7);
            Completion completion = new Completion(commandId, context, true);
            Class<?>[] types = method.getParameterTypes();
            method.invoke(
                    manager,
                    chatId,
                    callId,
                    hostV4,
                    hostV6,
                    port,
                    completion.callback(types[5], true),
                    completion.callback(types[6], false));
        } catch (Throwable error) {
            completeFailure(commandId, "join voice room", error);
        }
    }

    public static void leave(long commandId, String kind, long chatId) {
        try {
            Object context = application();
            Object vox = facade(context);
            if ("cecall".equals(kind)) {
                Object manager = invokeNoArgs(vox, "getCecallManager");
                exactMethod(manager.getClass(), "endCallByLeaveChatRoom", 1)
                        .invoke(manager, chatId);
                Bridge.complete(commandId, true, null);
                return;
            }
            if ("voiceroom".equals(kind)) {
                Object manager = invokeNoArgs(vox, "getVoiceroomManager");
                Method method = exactMethod(manager.getClass(), "leaveVoiceroom", 1);
                Completion completion = new Completion(commandId, null, false);
                method.invoke(manager, completion.callback(method.getParameterTypes()[0], true));
                return;
            }
            throw new IllegalArgumentException("kind must be cecall or voiceroom");
        } catch (Throwable error) {
            completeFailure(commandId, "leave VOX session", error);
        }
    }

    public static String status() {
        try {
            Object context = application();
            Object vox = facade(context);
            Object cecall = invokeNoArgs(vox, "getCecallManager");
            Object voiceroom = invokeNoArgs(vox, "getVoiceroomManager");
            boolean cecallIdle = (Boolean) invokeNoArgs(cecall, "isInIdle");
            boolean voiceroomIdle = (Boolean) invokeNoArgs(voiceroom, "isInIdle");
            Object callInfo = invokeNoArgs(cecall, "getCurrentCallInfo");
            Object roomInfo = invokeNoArgs(voiceroom, "getCurrentVoiceroomInfo");
            return "{"
                    + "\"moduleLoaded\":true,"
                    + "\"cecall\":" + cecallJson(cecallIdle, callInfo) + ","
                    + "\"voiceroom\":" + voiceroomJson(voiceroomIdle, roomInfo)
                    + "}";
        } catch (Throwable error) {
            return "{\"moduleLoaded\":false,\"error\":\""
                    + escape(rootCause(error).toString()) + "\"}";
        }
    }

    private static String cecallJson(boolean idle, Object info) throws Exception {
        if (info == null) {
            return "{\"idle\":" + idle
                    + ",\"chatId\":null,\"callId\":null,\"state\":null,\"type\":null}";
        }
        long callId = longProperty(info, "getCallId", 0);
        Object chatInfo = objectProperty(info, "getChatInfo", 1);
        long chatId = longProperty(chatInfo, "getChatRoomId", 0);
        String state = String.valueOf(objectProperty(info, "getCallState", 0));
        String type = String.valueOf(objectProperty(info, "getType", 2));
        return "{\"idle\":" + idle
                + ",\"chatId\":\"" + chatId + "\""
                + ",\"callId\":\"" + callId + "\""
                + ",\"state\":\"" + escape(state) + "\""
                + ",\"type\":\"" + escape(type) + "\"}";
    }

    private static String voiceroomJson(boolean idle, Object info) throws Exception {
        if (info == null) {
            return "{\"idle\":" + idle
                    + ",\"chatId\":null,\"callId\":null,\"screen\":null}";
        }
        long chatId = longProperty(info, "getChatRoomId", 0);
        long callId = longProperty(info, "getCallId", 1);
        String screen = String.valueOf(objectProperty(info, "getScreenType", 0));
        return "{\"idle\":" + idle
                + ",\"chatId\":\"" + chatId + "\""
                + ",\"callId\":\"" + callId + "\""
                + ",\"screen\":\"" + escape(screen) + "\"}";
    }

    private static Object facade(Object context) throws Exception {
        Object existing = facade;
        if (existing != null) return existing;
        synchronized (VoxController.class) {
            existing = facade;
            if (existing != null) return existing;
            ClassLoader loader = context.getClass().getClassLoader();
            Class<?> facadeType = load(loader, FACADE);
            Class<?> factoryType = load(loader, FACADE_FACTORY);
            Constructor<?> constructor = factoryType.getDeclaredConstructor();
            constructor.setAccessible(true);
            Object factory = constructor.newInstance();
            for (Method method : factoryType.getMethods()) {
                if (method.getParameterTypes().length == 1
                        && facadeType.isAssignableFrom(method.getReturnType())) {
                    method.setAccessible(true);
                    existing = method.invoke(factory, context);
                    if (existing != null) {
                        facade = existing;
                        return existing;
                    }
                }
            }
            throw new NoSuchMethodException("VoxModuleFacade factory method");
        }
    }

    private static Object application() throws Exception {
        Class<?> activityThread = Class.forName("android.app.ActivityThread");
        Method current = activityThread.getDeclaredMethod("currentApplication");
        current.setAccessible(true);
        Object application = current.invoke(null);
        if (application == null) throw new IllegalStateException("Application is not ready");
        return application;
    }

    private static void requireMicrophonePermission(Object context) throws Exception {
        Method check = context.getClass().getMethod("checkSelfPermission", String.class);
        int result = ((Number) check.invoke(context, "android.permission.RECORD_AUDIO")).intValue();
        if (result != 0) {
            throw new SecurityException("KakaoTalk RECORD_AUDIO permission is not granted");
        }
    }

    private static void openVoiceroom(Object context) throws Exception {
        ClassLoader loader = context.getClass().getClassLoader();
        Class<?> intentType = load(loader, "android.content.Intent");
        Class<?> contextType = load(loader, "android.content.Context");
        Constructor<?> constructor = intentType.getConstructor(contextType, Class.class);
        Object intent = constructor.newInstance(context, load(loader, VOICEROOM_ACTIVITY));
        intentType.getMethod("addFlags", int.class).invoke(intent, 0x30000000);
        context.getClass().getMethod("startActivity", intentType).invoke(context, intent);
    }

    private static Constructor<?> makingConstructor(Class<?> type) throws Exception {
        for (Constructor<?> constructor : type.getDeclaredConstructors()) {
            Class<?>[] parameters = constructor.getParameterTypes();
            if (parameters.length == 9
                    && parameters[0] == long.class
                    && parameters[1] == long.class
                    && parameters[2] == long.class
                    && List.class.isAssignableFrom(parameters[3])
                    && parameters[4].isEnum()) {
                constructor.setAccessible(true);
                return constructor;
            }
        }
        throw new NoSuchMethodException("CecallMakingInfo constructor");
    }

    private static Object enumConstant(Class<?> type, String name) {
        for (Object value : type.getEnumConstants()) {
            if (((Enum<?>) value).name().equals(name)) return value;
        }
        throw new IllegalArgumentException("enum constant not found: " + name);
    }

    private static Object singleton(Class<?> type) throws Exception {
        for (Field field : type.getDeclaredFields()) {
            if (Modifier.isStatic(field.getModifiers()) && type.isAssignableFrom(field.getType())) {
                field.setAccessible(true);
                Object value = field.get(null);
                if (value != null) return value;
            }
        }
        Constructor<?> constructor = type.getDeclaredConstructor();
        constructor.setAccessible(true);
        return constructor.newInstance();
    }

    private static List<Long> parsePeers(String value) {
        List<Long> peers = new ArrayList<>();
        if (value == null || value.isEmpty()) return peers;
        for (String item : value.split(",")) {
            long id = Long.parseLong(item);
            if (id <= 0) throw new IllegalArgumentException("peer ID must be positive");
            peers.add(id);
        }
        return peers;
    }

    private static Method exactMethod(Class<?> type, String name, int arity)
            throws NoSuchMethodException {
        for (Class<?> current = type; current != null; current = current.getSuperclass()) {
            for (Method method : current.getDeclaredMethods()) {
                if (method.getName().equals(name)
                        && method.getParameterTypes().length == arity) {
                    method.setAccessible(true);
                    return method;
                }
            }
        }
        for (Class<?> contract : type.getInterfaces()) {
            for (Method method : contract.getMethods()) {
                if (method.getName().equals(name)
                        && method.getParameterTypes().length == arity) {
                    method.setAccessible(true);
                    return method;
                }
            }
        }
        throw new NoSuchMethodException(type.getName() + "." + name + "/" + arity);
    }

    private static Object invokeNoArgs(Object target, String name) throws Exception {
        return exactMethod(target.getClass(), name, 0).invoke(target);
    }

    private static long longProperty(Object target, String getter, int ordinal)
            throws Exception {
        try {
            return number(invokeNoArgs(target, getter));
        } catch (NoSuchMethodException ignored) {
            return number(instanceField(target, long.class, ordinal));
        }
    }

    private static Object objectProperty(Object target, String getter, int ordinal)
            throws Exception {
        try {
            return invokeNoArgs(target, getter);
        } catch (NoSuchMethodException ignored) {
            return instanceField(target, null, ordinal);
        }
    }

    private static Object instanceField(Object target, Class<?> primitiveType, int ordinal)
            throws Exception {
        int index = 0;
        for (Class<?> current = target.getClass(); current != null;
                current = current.getSuperclass()) {
            for (Field field : current.getDeclaredFields()) {
                if (Modifier.isStatic(field.getModifiers())) continue;
                Class<?> fieldType = field.getType();
                boolean matches = primitiveType == null
                        ? !fieldType.isPrimitive()
                        : fieldType == primitiveType;
                if (!matches) continue;
                if (index++ == ordinal) {
                    field.setAccessible(true);
                    return field.get(target);
                }
            }
        }
        String kind = primitiveType == null ? "object" : primitiveType.getName();
        throw new NoSuchFieldException(
                target.getClass().getName() + " " + kind + " property #" + ordinal);
    }

    private static Class<?> load(ClassLoader loader, String name)
            throws ClassNotFoundException {
        return Class.forName(name, true, loader);
    }

    private static long number(Object value) {
        return value instanceof Number ? ((Number) value).longValue() : 0L;
    }

    private static void completeFailure(long commandId, String action, Throwable error) {
        Throwable cause = rootCause(error);
        Bridge.complete(commandId, false, action + " failed: " + cause);
    }

    private static Throwable rootCause(Throwable error) {
        Throwable result = error;
        while (result.getCause() != null && result.getCause() != result) {
            result = result.getCause();
        }
        return result;
    }

    private static String escape(String value) {
        return value.replace("\\", "\\\\")
                .replace("\"", "\\\"")
                .replace("\n", "\\n")
                .replace("\r", "\\r");
    }

    private static final class Completion {
        private final long commandId;
        private final Object context;
        private final boolean openOnSuccess;
        private final AtomicBoolean completed = new AtomicBoolean();

        Completion(long commandId, Object context, boolean openOnSuccess) {
            this.commandId = commandId;
            this.context = context;
            this.openOnSuccess = openOnSuccess;
        }

        Object callback(Class<?> type, boolean success) {
            return Proxy.newProxyInstance(type.getClassLoader(), new Class<?>[] {type},
                    new Callback(this, success));
        }

        void finish(boolean success, Object[] arguments) {
            if (!completed.compareAndSet(false, true)) return;
            if (success) {
                try {
                    if (openOnSuccess && context != null) openVoiceroom(context);
                    Bridge.complete(commandId, true, null);
                } catch (Throwable error) {
                    completeFailure(commandId, "open voice room UI", error);
                }
            } else {
                String reason = arguments == null || arguments.length == 0
                        ? "unknown reason" : String.valueOf(arguments[0]);
                Bridge.complete(commandId, false, "VOX request failed: " + reason);
            }
        }
    }

    private static final class Callback implements InvocationHandler {
        private final Completion completion;
        private final boolean success;

        Callback(Completion completion, boolean success) {
            this.completion = completion;
            this.success = success;
        }

        @Override
        public Object invoke(Object proxy, Method method, Object[] arguments) {
            if (method.getDeclaringClass() == Object.class) {
                if ("toString".equals(method.getName())) return "NoaVoxCallback";
                if ("hashCode".equals(method.getName())) return System.identityHashCode(proxy);
                if ("equals".equals(method.getName())) {
                    return arguments != null && arguments.length == 1 && proxy == arguments[0];
                }
            }
            if ("invoke".equals(method.getName())) completion.finish(success, arguments);
            return null;
        }
    }
}
