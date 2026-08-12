package dev.noa.kakao;

import Cs.e;
import com.kakao.talk.db.model.chatlog.d;
import com.kakao.talk.manager.send.m;

public final class SendListener implements m {
    private final long id;

    public SendListener(long id) {
        this.id = id;
    }

    @Override
    public void onCompleted(d chatLog, long chatRoomId, e sharedChatType) {
        Bridge.complete(id, true, null);
    }

    @Override
    public void onFailed(int status, String message) {
        Bridge.complete(id, false, "status=" + status + ", message=" + message);
    }

    @Override
    public void onException(Throwable error) {
        Bridge.complete(id, false, String.valueOf(error));
    }
}
