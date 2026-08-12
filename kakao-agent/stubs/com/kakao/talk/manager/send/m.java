package com.kakao.talk.manager.send;

import Cs.e;
import com.kakao.talk.db.model.chatlog.d;

public interface m {
    void onCompleted(d chatLog, long chatRoomId, e sharedChatType);
    void onFailed(int status, String message);
    void onException(Throwable error);
}
