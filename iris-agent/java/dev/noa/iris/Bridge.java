package dev.noa.iris;

import java.lang.reflect.Method;

public final class Bridge {
    private Bridge() {}

    static native Object invoke(int kind, Method backup, Object[] args);

    static native EndpointResponse endpoint(
            String method, String uri, String contentType, byte[] body);
}
