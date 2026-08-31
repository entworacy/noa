package dev.noa.kakao;

public final class MainDispatch implements Runnable {
    private final long id;
    private final int action;

    public MainDispatch(long id, int action) {
        this.id = id;
        this.action = action;
    }

    @Override
    public void run() {
        Bridge.dispatch(id, action);
    }
}
