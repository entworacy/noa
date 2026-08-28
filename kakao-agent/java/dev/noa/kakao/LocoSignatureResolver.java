package dev.noa.kakao;

import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Enumeration;
import java.util.HashSet;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;

/** Resolves LOCO hook targets from relationships that survive R8 renaming. */
public final class LocoSignatureResolver {
    private static final Profile[] PROFILES = {
        new Profile(29260630L, "gt.h"),
        new Profile(29260720L, "com.quram.mi.ocr.ig30"),
    };

    private LocoSignatureResolver() {}

    /** Returns send, receive, coroutine-resume methods and a diagnostic description. */
    public static Object[] resolve() throws Exception {
        Object application = currentApplication();
        Class<?> context = Class.forName("android.content.Context");
        ClassLoader loader = (ClassLoader) context.getMethod("getClassLoader").invoke(application);
        long versionCode = versionCode(application);
        List<String> failures = new ArrayList<>();
        Set<String> attempted = new LinkedHashSet<>();
        Set<String> classes = applicationClasses(application);

        // Profiles are only an acceleration hint. Every result still has to pass
        // the same relationship checks as an unknown version.
        for (Profile profile : PROFILES) {
            if (profile.versionCode == versionCode) {
                Resolution resolution = tryCandidate(
                        loader, profile.clientClass, classes, failures);
                attempted.add(profile.clientClass);
                if (resolution != null) {
                    return resolution.result(versionCode, "profile");
                }
            }
        }
        for (Profile profile : PROFILES) {
            if (!attempted.add(profile.clientClass)) {
                continue;
            }
            Resolution resolution = tryCandidate(
                    loader, profile.clientClass, classes, failures);
            if (resolution != null) {
                return resolution.result(versionCode, "known-signature");
            }
        }

        // Only outer classes that actually have inner classes can be LocoClient.
        // This avoids loading every class from a large, multidex KakaoTalk APK.
        Set<String> outers = new HashSet<>();
        for (String name : classes) {
            int separator = name.indexOf('$');
            if (separator > 0) {
                outers.add(name.substring(0, separator));
            }
        }
        List<String> names = new ArrayList<>(outers);
        Collections.sort(names);
        for (String name : names) {
            if (!attempted.add(name)) {
                continue;
            }
            Resolution resolution = tryCandidate(loader, name, classes, failures);
            if (resolution != null) {
                return resolution.result(versionCode, "dex-signature");
            }
        }
        String detail = failures.isEmpty()
                ? "no class had the required LocoClient method relationships"
                : failures.get(failures.size() - 1);
        throw new NoSuchMethodException(
                "LOCO signature was not found (versionCode=" + versionCode
                        + ", scanned=" + classes.size() + "): " + detail);
    }

    private static Resolution tryCandidate(
            ClassLoader loader, String name, Set<String> classes, List<String> failures) {
        try {
            Class<?> client = Class.forName(name, false, loader);
            Method[] methods = client.getDeclaredMethods();
            if (!Modifier.isFinal(client.getModifiers()) || methods.length < 10) {
                return null;
            }

            Resolution matched = null;
            for (Method factory : methods) {
                Class<?>[] parameters = factory.getParameterTypes();
                Class<?> receiver = factory.getReturnType();
                if (Modifier.isStatic(factory.getModifiers())
                        || parameters.length != 6
                        || !receiver.getName().startsWith(name + "$")
                        || parameters[2] != boolean.class
                        || parameters[3] != boolean.class
                        || !"kotlin.jvm.functions.Function2".equals(parameters[4].getName())
                        || !"kotlin.jvm.functions.Function0".equals(parameters[5].getName())) {
                    continue;
                }
                Class<?> request = parameters[0];
                Method send = uniqueSend(methods, request);
                Method receive = uniqueReceive(receiver);
                if (send == null || receive == null) {
                    continue;
                }
                Class<?> response = receive.getParameterTypes()[0];
                if (!packetRelationship(request, response)
                        || !hasStaticSendBridge(methods, client, request)
                        || !receiverBehavior(receiver)) {
                    continue;
                }
                Method resume = mainCoroutineResume(loader, name, client, classes);
                if (resume == null) {
                    continue;
                }
                if (matched != null) {
                    return null;
                }
                matched = new Resolution(send, receive, resume);
            }
            if (matched != null) {
                matched.send.setAccessible(true);
                matched.receive.setAccessible(true);
                matched.resume.setAccessible(true);
            }
            return matched;
        } catch (Throwable error) {
            if (failures.size() < 32) {
                failures.add(name + ": " + error);
            }
            return null;
        }
    }

    private static Method uniqueSend(Method[] methods, Class<?> request) {
        Method matched = null;
        for (Method method : methods) {
            Class<?>[] parameters = method.getParameterTypes();
            if (!Modifier.isStatic(method.getModifiers())
                    && method.getReturnType() == void.class
                    && parameters.length == 1
                    && parameters[0] == request) {
                if (matched != null) {
                    return null;
                }
                matched = method;
            }
        }
        return matched;
    }

    private static Method uniqueReceive(Class<?> receiver) {
        Method matched = null;
        for (Method method : receiver.getDeclaredMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            if (!Modifier.isStatic(method.getModifiers())
                    && method.getReturnType() == Object.class
                    && parameters.length == 2
                    && "kotlin.coroutines.Continuation".equals(parameters[1].getName())) {
                if (matched != null) {
                    return null;
                }
                matched = method;
            }
        }
        return matched;
    }

    private static boolean packetRelationship(Class<?> request, Class<?> response) {
        Package requestPackage = request.getPackage();
        Package responsePackage = response.getPackage();
        return Modifier.isFinal(request.getModifiers())
                && Modifier.isFinal(response.getModifiers())
                && request.getSuperclass() != null
                && request.getSuperclass() == response.getSuperclass()
                && request.getSuperclass() != Object.class
                && requestPackage != null
                && responsePackage != null
                && requestPackage.getName().equals(responsePackage.getName());
    }

    private static boolean hasStaticSendBridge(
            Method[] methods, Class<?> client, Class<?> request) {
        for (Method method : methods) {
            Class<?>[] parameters = method.getParameterTypes();
            if (Modifier.isStatic(method.getModifiers())
                    && method.getReturnType() == void.class
                    && parameters.length == 2
                    && parameters[0] == client
                    && parameters[1] == request) {
                return true;
            }
        }
        return false;
    }

    private static boolean receiverBehavior(Class<?> receiver) {
        boolean continuation = false;
        boolean noArgumentVoid = false;
        boolean noArgumentBoolean = false;
        for (Method method : receiver.getDeclaredMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            if (!Modifier.isStatic(method.getModifiers())
                    && method.getReturnType() == Object.class
                    && parameters.length == 1
                    && "kotlin.coroutines.Continuation".equals(parameters[0].getName())) {
                continuation = true;
            } else if (!Modifier.isStatic(method.getModifiers())
                    && method.getReturnType() == void.class
                    && parameters.length == 0) {
                noArgumentVoid = true;
            } else if (!Modifier.isStatic(method.getModifiers())
                    && method.getReturnType() == boolean.class
                    && parameters.length == 0) {
                noArgumentBoolean = true;
            }
        }
        return continuation && noArgumentVoid && noArgumentBoolean;
    }

    private static Method mainCoroutineResume(
            ClassLoader loader, String clientName, Class<?> client, Set<String> classes) {
        Set<String> innerNames = new LinkedHashSet<>();
        for (Class<?> inner : client.getDeclaredClasses()) {
            innerNames.add(inner.getName());
        }
        for (String name : classes) {
            if (name.startsWith(clientName + "$")) {
                innerNames.add(name);
            }
        }
        Method matched = null;
        int capturedState = -1;
        for (String innerName : innerNames) {
            try {
                Class<?> inner = Class.forName(innerName, false, loader);
                Method resume = invokeSuspend(inner);
                int fields = inner.getDeclaredFields().length;
                if (resume != null && fields > capturedState) {
                    matched = resume;
                    capturedState = fields;
                }
            } catch (Throwable ignored) {
            }
        }
        return matched;
    }

    private static Method invokeSuspend(Class<?> type) {
        for (Method method : type.getDeclaredMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            if ("invokeSuspend".equals(method.getName())
                    && !Modifier.isStatic(method.getModifiers())
                    && method.getReturnType() == Object.class
                    && parameters.length == 1
                    && parameters[0] == Object.class) {
                return method;
            }
        }
        return null;
    }

    private static Object currentApplication() throws Exception {
        Class<?> activityThread = Class.forName("android.app.ActivityThread");
        Object application = activityThread.getMethod("currentApplication").invoke(null);
        if (application == null) {
            throw new IllegalStateException("Android application is not ready");
        }
        return application;
    }

    private static long versionCode(Object application) {
        try {
            Class<?> context = Class.forName("android.content.Context");
            String packageName = (String) context.getMethod("getPackageName").invoke(application);
            Object manager = context.getMethod("getPackageManager").invoke(application);
            Method getter = Class.forName("android.content.pm.PackageManager")
                    .getMethod("getPackageInfo", String.class, int.class);
            Object info = getter.invoke(manager, packageName, 0);
            try {
                return ((Number) info.getClass().getMethod("getLongVersionCode").invoke(info))
                        .longValue();
            } catch (ReflectiveOperationException ignored) {
                return info.getClass().getField("versionCode").getInt(info);
            }
        } catch (Throwable ignored) {
            return -1L;
        }
    }

    private static Set<String> applicationClasses(Object application) throws Exception {
        Object info = Class.forName("android.content.Context")
                .getMethod("getApplicationInfo")
                .invoke(application);
        List<String> paths = new ArrayList<>();
        paths.add((String) publicField(info, "sourceDir"));
        Object splits = publicField(info, "splitSourceDirs");
        if (splits instanceof String[]) {
            Collections.addAll(paths, (String[]) splits);
        }

        Class<?> dexFileClass = Class.forName("dalvik.system.DexFile");
        Constructor<?> constructor = dexFileClass.getConstructor(String.class);
        Method entries = dexFileClass.getMethod("entries");
        Method close = dexFileClass.getMethod("close");
        Set<String> classes = new LinkedHashSet<>();
        for (String path : paths) {
            if (path == null || path.isEmpty()) {
                continue;
            }
            Object dexFile = null;
            try {
                dexFile = constructor.newInstance(path);
                @SuppressWarnings("unchecked")
                Enumeration<String> names = (Enumeration<String>) entries.invoke(dexFile);
                while (names.hasMoreElements()) {
                    classes.add(names.nextElement());
                }
            } catch (Throwable ignored) {
                // ABI and resource-only splits do not necessarily contain a DEX file.
            } finally {
                if (dexFile != null) {
                    try {
                        close.invoke(dexFile);
                    } catch (Throwable ignored) {
                    }
                }
            }
        }
        return classes;
    }

    private static Object publicField(Object target, String name) throws Exception {
        Field field = target.getClass().getField(name);
        return field.get(target);
    }

    private static final class Profile {
        final long versionCode;
        final String clientClass;

        Profile(long versionCode, String clientClass) {
            this.versionCode = versionCode;
            this.clientClass = clientClass;
        }
    }

    private static final class Resolution {
        final Method send;
        final Method receive;
        final Method resume;

        Resolution(Method send, Method receive, Method resume) {
            this.send = send;
            this.receive = receive;
            this.resume = resume;
        }

        Object[] result(long versionCode, String source) {
            return new Object[] {
                send,
                receive,
                resume,
                "versionCode=" + versionCode + ", source=" + source
                        + ", send=" + send.toGenericString()
                        + ", receive=" + receive.toGenericString()
                        + ", resume=" + resume.toGenericString(),
            };
        }
    }
}
