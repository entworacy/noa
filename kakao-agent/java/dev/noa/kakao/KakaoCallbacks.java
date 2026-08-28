package dev.noa.kakao;

import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;

/** Creates listener proxies without coupling Rust to renamed callback interfaces. */
final class KakaoCallbacks {
    private KakaoCallbacks() {}

    static Object completion(ClassLoader loader, Class<?> listener, long commandId) {
        return Proxy.newProxyInstance(loader, new Class<?>[] {listener},
                new CompletionHandler(commandId, listener));
    }

    static Object send(ClassLoader loader, Class<?> listener, long commandId) {
        return Proxy.newProxyInstance(loader, new Class<?>[] {listener},
                new SendCompletionHandler(commandId));
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
            successMethod = callbacks.isEmpty()
                    ? "" : callbacks.get(callbacks.size() - 1).getName();
        }

        @Override
        public Object invoke(Object proxy, Method method, Object[] arguments) {
            Object objectResult = objectMethod(proxy, method, arguments,
                    "NoaCompletionProxy(" + commandId + ")");
            if (objectResult != NOT_OBJECT_METHOD) return objectResult;
            boolean success = method.getName().equals(successMethod)
                    && (arguments == null || arguments.length == 0);
            Bridge.complete(commandId, success,
                    success ? null : callbackError(method, arguments));
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
            Object objectResult = objectMethod(proxy, method, arguments,
                    "NoaSendCompletionProxy(" + commandId + ")");
            if (objectResult != NOT_OBJECT_METHOD) return objectResult;
            Class<?>[] parameters = method.getParameterTypes();
            boolean success = parameters.length == 3 && parameters[1] == long.class;
            Bridge.complete(commandId, success,
                    success ? null : callbackError(method, arguments));
            return primitiveDefault(method.getReturnType());
        }
    }

    private static final Object NOT_OBJECT_METHOD = new Object();

    private static Object objectMethod(
            Object proxy, Method method, Object[] arguments, String description) {
        if (method.getDeclaringClass() != Object.class) return NOT_OBJECT_METHOD;
        if ("toString".equals(method.getName())) return description;
        if ("hashCode".equals(method.getName())) return System.identityHashCode(proxy);
        if ("equals".equals(method.getName())) {
            return arguments != null && arguments.length == 1 && proxy == arguments[0];
        }
        return primitiveDefault(method.getReturnType());
    }

    private static String callbackError(Method method, Object[] arguments) {
        StringBuilder message = new StringBuilder("KakaoTalk request failed: ")
                .append(method.getName());
        if (arguments != null && arguments.length > 0) {
            message.append(' ');
            for (int index = 0; index < arguments.length; index++) {
                if (index > 0) message.append(", ");
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
}
