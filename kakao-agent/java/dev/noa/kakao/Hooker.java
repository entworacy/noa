package dev.noa.kakao;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.util.Arrays;

public final class Hooker {
    private final int kind;
    private final boolean staticTarget;
    private final int packetIndex;
    public Method backup;

    public Hooker(int kind, boolean staticTarget, int packetIndex) {
        this.kind = kind;
        this.staticTarget = staticTarget;
        this.packetIndex = packetIndex;
    }

    public Object callback(Object[] args) throws Throwable {
        if (packetIndex >= 0 && packetIndex < args.length) {
            try {
                Bridge.capture(kind, args[packetIndex]);
            } catch (Throwable ignored) {
            }
        }
        Object receiver = staticTarget || args.length == 0 ? null : args[0];
        Object[] actual = staticTarget
                ? args
                : args.length < 2 ? new Object[0] : Arrays.copyOfRange(args, 1, args.length);
        try {
            return backup.invoke(receiver, actual);
        } catch (InvocationTargetException error) {
            throw error.getCause();
        }
    }
}
