package dev.noa.iris;

import java.lang.reflect.Method;

public final class Bridge {
    private Bridge() {}

    static native Object invoke(int kind, Method backup, Object[] args);
}
