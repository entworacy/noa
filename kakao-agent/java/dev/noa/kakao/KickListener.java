package dev.noa.kakao;

import iR.s;

public final class KickListener implements s.e {
    private final long id;

    public KickListener(long id) {
        this.id = id;
    }

    @Override
    public void a() {
        Bridge.complete(id, false, "KakaoTalk kick request failed");
    }

    @Override
    public void b() {
        Bridge.complete(id, true, null);
    }
}
