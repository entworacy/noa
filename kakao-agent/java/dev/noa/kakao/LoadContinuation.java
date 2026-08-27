package dev.noa.kakao;

import kotlin.coroutines.Continuation;
import kotlin.coroutines.CoroutineContext;

public final class LoadContinuation implements Continuation<Object> {
    private final long id;
    private final CoroutineContext context;

    public LoadContinuation(long id, CoroutineContext context) {
        this.id = id;
        this.context = context;
    }

    @Override
    public CoroutineContext getContext() {
        return context;
    }

    @Override
    public void resumeWith(Object result) {
        Bridge.loaded(id);
    }
}
