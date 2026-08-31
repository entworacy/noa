package dev.noa.kakao;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.nio.ByteBuffer;
import java.util.Arrays;

/** Rewrites only VOX WebRTC microphone frames after AudioRecord has filled them. */
public final class VoxAudioHooker {
    private static final String VOX_AUDIO_THREAD = "AudioRecordJavaThread";

    public Method backup;

    public Object callback(Object[] args) throws Throwable {
        Object receiver = args.length == 0 ? null : args[0];
        Object[] actual = args.length < 2
                ? new Object[0] : Arrays.copyOfRange(args, 1, args.length);
        Object result;
        try {
            result = backup.invoke(receiver, actual);
        } catch (InvocationTargetException error) {
            throw error.getCause();
        }

        if (result instanceof Number
                && ((Number) result).intValue() > 0
                && args.length >= 3
                && args[1] instanceof ByteBuffer
                && Thread.currentThread().getName().startsWith(VOX_AUDIO_THREAD)) {
            try {
                Bridge.processVoxAudio(
                        (ByteBuffer) args[1], ((Number) result).intValue());
            } catch (Throwable ignored) {
            }
        }
        return result;
    }
}
