package dev.noa.kakao;

import java.lang.ref.WeakReference;
import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.Map;
import java.util.concurrent.atomic.AtomicInteger;

/** Sends through the observed app client and its normal packet builder/ID allocator. */
public final class LocoSender {
    private static volatile WeakReference<Object> observed = new WeakReference<>(null);
    private static volatile Binding binding;

    private LocoSender() {}

    static void observe(Object client) {
        if (observed.get() != client) observed = new WeakReference<>(client);
    }

    public static long[] send(String method, Map<String, Object> body) throws Exception {
        if (method == null || !method.matches("[A-Z0-9]{1,11}") || body == null) {
            throw new IllegalArgumentException("invalid LOCO method/body");
        }
        Binding b = resolve();
        Object client = observed.get();
        if (client == null || !b.send.getDeclaringClass().isInstance(client)) {
            throw new IllegalStateException("LOCO client not observed yet; wait for normal app traffic");
        }
        Object command = null;
        for (Object value : b.methodType.getEnumConstants()) {
            if (((Enum<?>) value).name().equals(method)) command = value;
        }
        if (command == null) throw new IllegalArgumentException("unknown LOCO method: " + method);
        // Hooker uses this same monitor for every ordinary app write.
        synchronized (client) {
            if (observed.get() != client || !KakaoLocoStateResolver.connected()
                    || !Boolean.TRUE.equals(b.alive.invoke(client))) {
                throw new IllegalStateException("LOCO client is disconnected or changed");
            }
            int id = ((Number) b.nextId.invoke(b.idOwner)).intValue();
            Object builder = b.builder.newInstance(command, id, (short) 0);
            for (Map.Entry<String, Object> entry : body.entrySet()) {
                b.put.invoke(builder, entry.getKey(), entry.getValue());
            }
            Object packet = b.build.invoke(builder);
            Object[] fields = LocoPacketReader.read(packet);
            if (((Number) fields[0]).intValue() != id || !command.equals(fields[2])) {
                throw new IllegalStateException("LOCO builder returned mismatched packet");
            }
            long length = ((Number) fields[3]).longValue();
            if (length < 5 || length > 131072) throw new IllegalArgumentException("invalid BSON body size");
            b.send.invoke(client, packet);
            return new long[] { id, length };
        }
    }

    private static Binding resolve() throws Exception {
        Binding result = binding;
        if (result != null) return result;
        synchronized (LocoSender.class) {
            if (binding == null) binding = new Binding((Method) LocoSignatureResolver.resolve()[0]);
            return binding;
        }
    }

    private static Method unique(Class<?> owner, Class<?> result, Class<?>... args) throws Exception {
        Method found = null;
        for (Method method : owner.getDeclaredMethods()) {
            if (!Modifier.isStatic(method.getModifiers()) && method.getReturnType() == result
                    && java.util.Arrays.equals(method.getParameterTypes(), args)) {
                if (found != null) throw new NoSuchMethodException("ambiguous method on " + owner.getName());
                found = method;
            }
        }
        if (found == null) throw new NoSuchMethodException("missing method on " + owner.getName());
        found.setAccessible(true);
        return found;
    }

    private static final class Binding {
        final Method send, alive, put, build, nextId;
        final Class<?> methodType;
        final Constructor<?> builder;
        final Object idOwner;

        Binding(Method send) throws Exception {
            this.send = send;
            send.setAccessible(true);
            Class<?> request = send.getParameterTypes()[0];
            methodType = LocoPacketReader.methodType(request);
            alive = unique(send.getDeclaringClass(), boolean.class);
            Constructor<?> ctor = null;
            for (Class<?> type : KakaoSignatureResolver.sourceClasses("LocoReq.kt")) {
                try {
                    Constructor<?> candidate = type.getDeclaredConstructor(methodType, int.class, short.class);
                    unique(type, request);
                    unique(type, type, String.class, Object.class);
                    if (ctor != null) throw new IllegalStateException("ambiguous LOCO request builder");
                    ctor = candidate;
                } catch (NoSuchMethodException ignored) {}
            }
            if (ctor == null) throw new NoSuchMethodException("LOCO request builder not found");
            builder = ctor;
            builder.setAccessible(true);
            build = unique(builder.getDeclaringClass(), request);
            put = unique(builder.getDeclaringClass(), builder.getDeclaringClass(), String.class, Object.class);
            Method generator = null;
            Object owner = null;
            for (Class<?> type : KakaoSignatureResolver.sourceClasses("LocoJob.kt")) {
                boolean atomic = false;
                for (Field field : type.getDeclaredFields()) {
                    if (Modifier.isStatic(field.getModifiers()) && field.getType() == AtomicInteger.class) atomic = true;
                }
                if (!atomic) continue;
                Method candidate = unique(type, int.class);
                for (Object singleton : KakaoReflection.singletonObjects(type)) {
                    if (generator != null) throw new NoSuchMethodException("ambiguous LOCO ID allocator");
                    generator = candidate;
                    owner = singleton;
                }
            }
            if (generator == null) throw new NoSuchMethodException("LOCO ID allocator not found");
            nextId = generator;
            idOwner = owner;
        }
    }
}
