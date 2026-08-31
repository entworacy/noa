package dev.noa.kakao;

import java.lang.reflect.Field;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.lang.reflect.Proxy;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.Callable;
import java.util.concurrent.atomic.AtomicLong;

/** Connects to Room without referring to names changed by R8. */
public final class RoomWatcher {
    private static final List<Object> RETAINED = new ArrayList<>();

    private RoomWatcher() {}

    public static synchronized void install() throws Exception {
        if (!RETAINED.isEmpty()) {
            return;
        }
        attachDatabase(
                "master",
                "com.kakao.talk.database.MasterDatabase",
                new String[] {"chat_logs", "chat_rooms"});
        attachDatabase(
                "secondary",
                "com.kakao.talk.database.SecondaryDatabase",
                new String[] {"open_chat_member", "open_link", "open_profile"});
    }

    private static void attachDatabase(String label, String className, String[] tables)
            throws Exception {
        ClassLoader loader = RoomWatcher.class.getClassLoader();
        Class<?> databaseClass = Class.forName(className, true, loader);
        Object database = databaseInstance(databaseClass);
        Object tracker = invalidationTracker(database);
        for (String table : tables) {
            attachTable(label, table, tracker);
        }
    }

    private static Object databaseInstance(Class<?> databaseClass) throws Exception {
        for (Method method : databaseClass.getDeclaredMethods()) {
            if (Modifier.isStatic(method.getModifiers())
                    && method.getParameterTypes().length == 0
                    && databaseClass.isAssignableFrom(method.getReturnType())) {
                method.setAccessible(true);
                Object result = method.invoke(null);
                if (result != null) {
                    return result;
                }
            }
        }
        for (Field field : databaseClass.getDeclaredFields()) {
            if (!Modifier.isStatic(field.getModifiers())) {
                continue;
            }
            field.setAccessible(true);
            Object companion = field.get(null);
            if (companion == null) {
                continue;
            }
            for (Method method : companion.getClass().getDeclaredMethods()) {
                if (method.getParameterTypes().length == 0
                        && databaseClass.isAssignableFrom(method.getReturnType())) {
                    method.setAccessible(true);
                    Object result = method.invoke(companion);
                    if (result != null) {
                        return result;
                    }
                }
            }
        }
        throw new NoSuchMethodException("Room database singleton getter: " + className(databaseClass));
    }

    private static Object invalidationTracker(Object database) throws Exception {
        for (Class<?> owner = database.getClass(); owner != null; owner = owner.getSuperclass()) {
            for (Method method : owner.getDeclaredMethods()) {
                if (Modifier.isStatic(method.getModifiers())
                        || method.getParameterTypes().length != 0
                        || method.getReturnType().isPrimitive()
                        || !hasLiveDataFactory(method.getReturnType())) {
                    continue;
                }
                method.setAccessible(true);
                Object tracker = method.invoke(database);
                if (tracker != null) {
                    return tracker;
                }
            }
        }
        throw new NoSuchMethodException("Room invalidation tracker getter: " + className(database.getClass()));
    }

    private static boolean hasLiveDataFactory(Class<?> type) {
        for (Method method : type.getMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            if (parameters.length == 3
                    && parameters[0] == String[].class
                    && parameters[1] == boolean.class
                    && Callable.class.isAssignableFrom(parameters[2])
                    && method.getReturnType() != void.class) {
                return true;
            }
        }
        return false;
    }

    private static void attachTable(final String database, final String table, Object tracker)
            throws Exception {
        Method factory = null;
        for (Method method : tracker.getClass().getMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            if (parameters.length == 3
                    && parameters[0] == String[].class
                    && parameters[1] == boolean.class
                    && Callable.class.isAssignableFrom(parameters[2])
                    && method.getReturnType() != void.class) {
                factory = method;
                break;
            }
        }
        if (factory == null) {
            throw new NoSuchMethodException("Room LiveData factory: " + className(tracker.getClass()));
        }
        final AtomicLong version = new AtomicLong();
        Object liveData = factory.invoke(
                tracker,
                new String[] {table},
                false,
                (Callable<Long>) version::incrementAndGet);
        if (liveData == null) {
            throw new IllegalStateException("Room LiveData factory returned null for " + table);
        }
        attachForeverObserver(database, table, liveData);
    }

    private static void attachForeverObserver(
            final String database, final String table, Object liveData) throws Exception {
        List<Method> candidates = new ArrayList<>();
        for (Method method : liveData.getClass().getMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            if (method.getReturnType() == void.class
                    && parameters.length == 1
                    && parameters[0].isInterface()
                    && isValueObserver(parameters[0])) {
                candidates.add(method);
            }
        }
        if (candidates.isEmpty()) {
            throw new NoSuchMethodException("LiveData observer registration: " + className(liveData.getClass()));
        }

        final boolean[] initialized = {false};
        InvocationHandler callback = (proxy, method, args) -> {
            if (method.getDeclaringClass() == Object.class) {
                if ("hashCode".equals(method.getName())) return System.identityHashCode(proxy);
                if ("equals".equals(method.getName())) return proxy == args[0];
                return "NoaRoomObserver(" + database + ":" + table + ")";
            }
            if (initialized[0]) {
                Bridge.databaseInvalidated(database, table);
            } else {
                initialized[0] = true;
            }
            return null;
        };
        Class<?> observerType = candidates.get(0).getParameterTypes()[0];
        Object observer = Proxy.newProxyInstance(
                observerType.getClassLoader(), new Class<?>[] {observerType}, callback);

        // A fresh LiveData has no active observers. The add method is the candidate
        // that changes one of its zero-argument boolean state queries to true.
        Map<String, Boolean> before = booleanState(liveData);
        Method registration = null;
        for (Method candidate : candidates) {
            candidate.setAccessible(true);
            candidate.invoke(liveData, observer);
            if (becameTrue(before, booleanState(liveData))) {
                registration = candidate;
                break;
            }
        }
        if (registration == null) {
            throw new IllegalStateException("LiveData observer add method was not identified by behavior");
        }
        RETAINED.add(liveData);
        RETAINED.add(observer);
    }

    private static boolean isValueObserver(Class<?> type) {
        int abstractMethods = 0;
        for (Method method : type.getMethods()) {
            if (method.getDeclaringClass() == Object.class || !Modifier.isAbstract(method.getModifiers())) {
                continue;
            }
            Class<?>[] parameters = method.getParameterTypes();
            if (method.getReturnType() != void.class || parameters.length != 1) {
                return false;
            }
            abstractMethods++;
        }
        return abstractMethods == 1;
    }

    private static Map<String, Boolean> booleanState(Object target) throws Exception {
        Map<String, Boolean> values = new HashMap<>();
        for (Method method : target.getClass().getMethods()) {
            if (method.getParameterTypes().length == 0 && method.getReturnType() == boolean.class) {
                method.setAccessible(true);
                values.put(method.toGenericString(), (Boolean) method.invoke(target));
            }
        }
        return values;
    }

    private static boolean becameTrue(Map<String, Boolean> before, Map<String, Boolean> after) {
        for (Map.Entry<String, Boolean> entry : after.entrySet()) {
            if (!before.getOrDefault(entry.getKey(), false) && entry.getValue()) {
                return true;
            }
        }
        return false;
    }

    private static String className(Class<?> type) {
        return type == null ? "<null>" : type.getName();
    }
}
