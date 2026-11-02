package dev.noa.kakao;

import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.util.Arrays;
import java.util.HashSet;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

/** Reads the validated LOCO packet layout without embedding R8 class names. */
public final class LocoPacketReader {
    private static final ConcurrentHashMap<Class<?>, Binding> CACHE = new ConcurrentHashMap<>();

    private LocoPacketReader() {}

    public static void verify(Class<?> packet) throws Exception {
        binding(packet);
    }

    static Class<?> methodType(Class<?> packet) throws Exception {
        return binding(packet).method.getType();
    }

    public static Object[] read(Object packet) throws Exception {
        if (packet == null) throw new NullPointerException("LOCO packet is null");
        Binding b = binding(packet.getClass());
        Object header = b.header.get(packet);
        Object body = b.body.get(packet);
        if (header == null || body == null) throw new IllegalStateException("LOCO header/body is null");
        return new Object[] {
            b.id.get(header), b.status.get(header), b.method.get(header),
            b.length.get(header), b.bson.get(body),
        };
    }

    private static Binding binding(Class<?> packet) throws Exception {
        Binding existing = CACHE.get(packet);
        if (existing != null) return existing;
        Binding found = new Binding(packet);
        Binding raced = CACHE.putIfAbsent(packet, found);
        return raced == null ? found : raced;
    }

    private static Field sourceField(Class<?> packet, String source) throws Exception {
        Set<String> names = new HashSet<>(Arrays.asList(
                KakaoSignatureResolver.sourceClassNames(source)));
        Field found = null;
        for (Class<?> type = packet; type != null; type = type.getSuperclass()) {
            for (Field field : type.getDeclaredFields()) {
                if (Modifier.isStatic(field.getModifiers()) || !names.contains(field.getType().getName())) continue;
                if (found != null) throw new NoSuchFieldException("ambiguous LOCO " + source);
                found = field;
            }
        }
        if (found == null) throw new NoSuchFieldException("missing LOCO " + source);
        found.setAccessible(true);
        return found;
    }

    private static Field field(Class<?> owner, String name, Class<?> expected) throws Exception {
        Field field = owner.getDeclaredField(name);
        if (Modifier.isStatic(field.getModifiers()) || field.getType() != expected) {
            throw new NoSuchFieldException("unsupported LOCO field: " + owner.getName() + "." + name);
        }
        field.setAccessible(true);
        return field;
    }

    private static final class Binding {
        final Field header, body, id, status, method, length, bson;

        Binding(Class<?> packet) throws Exception {
            header = sourceField(packet, "LocoHeader.kt");
            body = sourceField(packet, "LocoBody.kt");
            Class<?> h = header.getType();
            Class<?> methodType = h.getDeclaredField("c").getType();
            if (!methodType.isEnum()) throw new NoSuchFieldException("LOCO method is not an enum");
            // Both audited APKs use this constructor and field layout; fail on drift.
            h.getDeclaredConstructor(int.class, short.class, methodType, int.class);
            id = field(h, "a", int.class);
            status = field(h, "b", short.class);
            method = field(h, "c", methodType);
            length = field(h, "d", int.class);
            Class<?> bodyType = body.getType();
            Class<?> bsonType = bodyType.getDeclaredField("a").getType();
            if (bsonType.isPrimitive()) throw new NoSuchFieldException("LOCO BSON is primitive");
            bodyType.getDeclaredConstructor(bsonType);
            bson = field(bodyType, "a", bsonType);
        }
    }
}
