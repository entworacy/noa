package dev.noa.iris;

import java.lang.reflect.InvocationHandler;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.util.Collections;
import java.util.IdentityHashMap;
import java.util.Set;

/** Installs Noa's endpoint gateway without linking the agent DEX to a specific Ktor release. */
public final class EndpointInstaller {
    private static final Set<Object> INSTALLED =
            Collections.newSetFromMap(new IdentityHashMap<Object, Boolean>());

    private static final Class<?> FUNCTION_1 = loadClass("kotlin.jvm.functions.Function1");
    private static final Class<?> FUNCTION_2 = loadClass("kotlin.jvm.functions.Function2");
    private static final Class<?> CONTINUATION = loadClass("kotlin.coroutines.Continuation");
    private static final Object UNIT = staticField("kotlin.Unit", "INSTANCE");
    private static final Object SUSPENDED = invokeStatic(
            method("kotlin.coroutines.intrinsics.IntrinsicsKt", "getCOROUTINE_SUSPENDED", 0));
    private static final Method RECEIVE_CHANNEL = method(
            "io.ktor.server.request.ApplicationReceiveFunctionsKt", "receiveChannel", 2);
    private static final Method READ_BYTES = method(
            "io.ktor.utils.io.ByteReadChannelOperationsKt", "toByteArray", 2);
    private static final Method RESPOND_TEXT = respondTextMethod();
    private static final Method GET_URI = method(
            "io.ktor.server.request.ApplicationRequestPropertiesKt", "getUri", 1);
    private static final Method GET_HTTP_METHOD = method(
            "io.ktor.server.request.ApplicationRequestPropertiesKt", "getHttpMethod", 1);
    private static final Method RESULT_FAILURE = method("kotlin.ResultKt", "createFailure", 1);
    private static final Method THROW_ON_FAILURE = method("kotlin.ResultKt", "throwOnFailure", 1);
    private static final Method ROUTE = routeMethod();

    private EndpointInstaller() {}

    public static boolean matchesPrefix(Object pipelineContext, String prefix) throws Exception {
        String path = requestPath(pipelineContext);
        return path != null && (path.equals(prefix) || path.startsWith(prefix + "/"));
    }

    public static String requestPath(Object pipelineContext) throws Exception {
        if (pipelineContext == null) {
            return null;
        }
        Object call = invoke(findMethod(pipelineContext.getClass(), "getContext", 0), pipelineContext);
        if (call == null) {
            return null;
        }
        Object request = invoke(findMethod(call.getClass(), "getRequest", 0), call);
        if (request == null) {
            return null;
        }
        String uri = (String) invoke(GET_URI, null, request);
        if (uri == null || uri.isEmpty()) {
            return null;
        }
        String path = new java.net.URI(uri).getPath();
        return path == null || path.isEmpty() ? "/" : path;
    }

    public static Object handleFromPipeline(Object pipelineContext, Object completion)
            throws Exception {
        Object call = invoke(findMethod(pipelineContext.getClass(), "getContext", 0), pipelineContext);
        return handleCall(call, completion);
    }

    public static void installFromCall(Object call, String prefix) throws Exception {
        Object application = invoke(findMethod(call.getClass(), "getApplication", 0), call);
        Class<?> routingRoot = loadClass("io.ktor.server.routing.RoutingRoot");
        Object plugin = routingRoot.getField("Plugin").get(null);
        Method lookup = null;
        for (Method candidate : loadClass(
                "io.ktor.server.application.ApplicationPluginKt").getMethods()) {
            Class<?>[] parameters = candidate.getParameterTypes();
            if (candidate.getName().equals("plugin")
                    && parameters.length == 2
                    && parameters[1].isInstance(plugin)) {
                lookup = candidate;
                break;
            }
        }
        if (lookup == null) {
            throw new IllegalStateException("compatible Ktor plugin lookup was not found");
        }
        install(invoke(lookup, null, application, plugin), prefix);
    }

    public static synchronized void install(Object routingRoot, String prefix) throws Exception {
        if (INSTALLED.contains(routingRoot)) {
            return;
        }

        Object configure = function(FUNCTION_1, (proxy, method, args) -> {
            if (isInvoke(method)) {
                attachHandler(args[0]);
                return UNIT;
            }
            return objectMethod(proxy, method, args);
        });
        invoke(ROUTE, null, routingRoot, prefix + "/{path...}", configure);
        INSTALLED.add(routingRoot);
    }

    private static void attachHandler(Object route) throws Exception {
        Method handle = findMethod(route.getClass(), "handle", 1);
        Object handler = function(FUNCTION_2, (proxy, method, args) -> {
            if (isInvoke(method)) {
                return handleRequest(args[0], args[1]);
            }
            return objectMethod(proxy, method, args);
        });
        invoke(handle, route, handler);
    }

    private static Object handleRequest(Object routingContext, Object completion) throws Exception {
        Object call = invoke(findMethod(routingContext.getClass(), "getCall", 0), routingContext);
        return handleCall(call, completion);
    }

    private static Object handleCall(Object call, Object completion) throws Exception {
        Object waiting = continuation(completion, result -> {
            throwOnFailure(result);
            receiveBody(call, result, completion, true);
        });
        Object received = invoke(RECEIVE_CHANNEL, null, call, waiting);
        if (received == SUSPENDED) {
            return SUSPENDED;
        }
        throwOnFailure(received);
        return receiveBody(call, received, completion, false);
    }

    private static Object receiveBody(
            Object call, Object channel, Object completion, boolean resumeCompletion)
            throws Exception {
        Object waiting = continuation(completion, result -> {
            throwOnFailure(result);
            finishRequest(call, (byte[]) result, completion, true);
        });
        Object received = invoke(READ_BYTES, null, channel, waiting);
        if (received == SUSPENDED) {
            return SUSPENDED;
        }
        throwOnFailure(received);
        return finishRequest(call, (byte[]) received, completion, resumeCompletion);
    }

    private static Object finishRequest(
            Object call, byte[] body, Object completion, boolean resumeCompletion) throws Exception {
        try {
            Object request = invoke(findMethod(call.getClass(), "getRequest", 0), call);
            Object httpMethod = invoke(GET_HTTP_METHOD, null, request);
            String method = (String) invoke(findMethod(httpMethod.getClass(), "getValue", 0), httpMethod);
            String uri = (String) invoke(GET_URI, null, request);
            Object headers = invoke(findMethod(request.getClass(), "getHeaders", 0), request);
            Object header = invoke(findStringMethod(headers.getClass(), "get"), headers, "Content-Type");
            String contentType = header == null ? "application/octet-stream" : String.valueOf(header);
            EndpointResponse response = Bridge.endpoint(method, uri, contentType, body);
            Object responseContentType = parseWithCompanion("io.ktor.http.ContentType", response.getContentType());
            Object status = parseStatus(response.getStatus());
            Object configure = function(FUNCTION_1, (proxy, called, args) ->
                    isInvoke(called) ? UNIT : objectMethod(proxy, called, args));
            Object sent = invoke(
                    RESPOND_TEXT,
                    null,
                    call,
                    response.getBody(),
                    responseContentType,
                    status,
                    configure,
                    completion);
            if (resumeCompletion && sent != SUSPENDED) {
                resume(completion, UNIT);
            }
            return sent;
        } catch (Throwable error) {
            if (!resumeCompletion) {
                if (error instanceof Exception) {
                    throw (Exception) error;
                }
                if (error instanceof Error) {
                    throw (Error) error;
                }
                throw new RuntimeException(error);
            }
            resume(completion, invoke(RESULT_FAILURE, null, unwrap(error)));
            return UNIT;
        }
    }

    private static Object continuation(Object completion, ResultHandler handler) {
        return function(CONTINUATION, (proxy, method, args) -> {
            if (method.getName().equals("getContext")) {
                return invoke(findMethod(completion.getClass(), "getContext", 0), completion);
            }
            if (method.getName().equals("resumeWith")) {
                handler.accept(args[0]);
                return null;
            }
            return objectMethod(proxy, method, args);
        });
    }

    private static void resume(Object continuation, Object result) throws Exception {
        invoke(findMethod(continuation.getClass(), "resumeWith", 1), continuation, result);
    }

    private static void throwOnFailure(Object result) throws Exception {
        invoke(THROW_ON_FAILURE, null, result);
    }

    private static Object parseWithCompanion(String className, String value) throws Exception {
        Class<?> type = loadClass(className);
        Object companion = type.getField("Companion").get(null);
        return invoke(findMethod(companion.getClass(), "parse", 1), companion, value);
    }

    private static Object parseStatus(int value) throws Exception {
        Class<?> type = loadClass("io.ktor.http.HttpStatusCode");
        Object companion = type.getField("Companion").get(null);
        return invoke(findMethod(companion.getClass(), "fromValue", 1), companion, value);
    }

    private static Method routeMethod() {
        for (Method candidate : loadClass("io.ktor.server.routing.RoutingBuilderKt").getMethods()) {
            Class<?>[] parameters = candidate.getParameterTypes();
            if (candidate.getName().equals("route")
                    && parameters.length == 3
                    && parameters[1] == String.class
                    && parameters[2].getName().equals("kotlin.jvm.functions.Function1")) {
                return candidate;
            }
        }
        throw new IllegalStateException("compatible Ktor route method was not found");
    }

    private static Method respondTextMethod() {
        for (Method candidate : loadClass(
                "io.ktor.server.response.ApplicationResponseFunctionsKt").getMethods()) {
            Class<?>[] parameters = candidate.getParameterTypes();
            if (candidate.getName().equals("respondText")
                    && parameters.length == 6
                    && parameters[1] == String.class) {
                return candidate;
            }
        }
        throw new IllegalStateException("compatible Ktor respondText method was not found");
    }

    private static Method method(String className, String name, int parameterCount) {
        return findMethod(loadClass(className), name, parameterCount);
    }

    private static Method findMethod(Class<?> type, String name, int parameterCount) {
        for (Method candidate : type.getMethods()) {
            if (candidate.getName().equals(name)
                    && candidate.getParameterTypes().length == parameterCount) {
                candidate.setAccessible(true);
                return candidate;
            }
        }
        throw new IllegalStateException(type.getName() + "." + name + " was not found");
    }

    private static Method findStringMethod(Class<?> type, String name) {
        for (Method candidate : type.getMethods()) {
            Class<?>[] parameters = candidate.getParameterTypes();
            if (candidate.getName().equals(name)
                    && parameters.length == 1
                    && parameters[0] == String.class) {
                candidate.setAccessible(true);
                return candidate;
            }
        }
        throw new IllegalStateException(type.getName() + "." + name + "(String) was not found");
    }

    private static Object staticField(String className, String name) {
        try {
            return loadClass(className).getField(name).get(null);
        } catch (ReflectiveOperationException error) {
            throw new IllegalStateException(error);
        }
    }

    private static Class<?> loadClass(String name) {
        try {
            return Class.forName(name);
        } catch (ClassNotFoundException error) {
            throw new IllegalStateException(error);
        }
    }

    private static Object function(Class<?> type, InvocationHandler handler) {
        return Proxy.newProxyInstance(type.getClassLoader(), new Class<?>[]{type}, handler);
    }

    private static boolean isInvoke(Method method) {
        return method.getName().equals("invoke");
    }

    private static Object objectMethod(Object proxy, Method method, Object[] args) {
        if (method.getName().equals("toString")) {
            return "NoaEndpointProxy";
        }
        if (method.getName().equals("hashCode")) {
            return System.identityHashCode(proxy);
        }
        if (method.getName().equals("equals")) {
            return proxy == args[0];
        }
        return null;
    }

    private static Object invokeStatic(Method method, Object... args) {
        try {
            return invoke(method, null, args);
        } catch (Exception error) {
            throw new IllegalStateException(error);
        }
    }

    private static Object invoke(Method method, Object receiver, Object... args) throws Exception {
        try {
            return method.invoke(receiver, args);
        } catch (InvocationTargetException error) {
            Throwable cause = error.getCause();
            if (cause instanceof Exception) {
                throw (Exception) cause;
            }
            if (cause instanceof Error) {
                throw (Error) cause;
            }
            throw error;
        }
    }

    private static Throwable unwrap(Throwable error) {
        return error instanceof InvocationTargetException && error.getCause() != null
                ? error.getCause()
                : error;
    }

    private interface ResultHandler {
        void accept(Object result) throws Exception;
    }
}
