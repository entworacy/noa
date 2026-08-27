package dev.noa.iris;

import java.lang.reflect.Method;

public final class Hooker {
    private final int kind;
    public Method backup;

    public Hooker(int kind) {
        this.kind = kind;
    }

    public Object callback(Object[] args) {
        return Bridge.invoke(kind, backup, args);
    }
}
