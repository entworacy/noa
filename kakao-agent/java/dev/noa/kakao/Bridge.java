package dev.noa.kakao;

import java.nio.ByteBuffer;

public final class Bridge {
    private Bridge() {}

    static native void loaded(long id);
    static native void complete(long id, boolean ok, String error);
    static native void dispatch(long id, int action);
    static native void capture(int kind, Object packet);
    static native void databaseInvalidated(String database, String table);
    static native void processVoxAudio(ByteBuffer buffer, int size);
}
