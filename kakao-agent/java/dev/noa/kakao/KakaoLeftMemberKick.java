package dev.noa.kakao;

import java.lang.reflect.Method;
import java.lang.reflect.Modifier;

/** KICKMEM without a GETMEM lookup, for a user who has already left the room. */
final class KakaoLeftMemberKick {
    private KakaoLeftMemberKick() {}

    static void kick(long roomId, long userId, long commandId) throws Exception {
        if (roomId <= 0 || userId <= 0) {
            throw new IllegalArgumentException("positive room and user IDs are required");
        }
        Object room = KakaoRoomResolver.find(roomId);
        ClassLoader loader = KakaoSignatureResolver.appLoader();
        Class<?> friend = loader.loadClass("com.kakao.talk.db.model.Friend");
        // The native kick coroutine reads only Friend.getId(). The profile viewer
        // also uses its retained Friend when the current member lookup is empty.
        Object member = friend.getConstructor(long.class, String.class, String.class)
                .newInstance(userId, "", "");
        if (!KakaoReflection.hasLongValue(member, userId)) {
            throw new IllegalStateException("kick target ID does not match");
        }
        Object manager = KakaoClassResolver.objectFor("open-link-manager");
        Method accessor = null;
        Method kick = null;
        for (Method method : manager.getClass().getDeclaredMethods()) {
            if (Modifier.isStatic(method.getModifiers())
                    || method.isBridge() || method.isSynthetic()
                    || method.getReturnType().isInterface()
                    || method.getParameterTypes().length != 0) continue;
            Method candidate = KakaoRoomResolver.kickMethod(
                    method.getReturnType(), room.getClass(), friend);
            if (candidate == null) continue;
            if (accessor != null) {
                throw new NoSuchMethodException("ambiguous open-chat kick accessor");
            }
            accessor = method;
            kick = candidate;
        }
        if (kick == null) {
            throw new NoSuchMethodException("open-chat kick signature was not found");
        }
        accessor.setAccessible(true);
        kick.setAccessible(true);
        Object foreground = accessor.invoke(manager);
        if (foreground == null) throw new IllegalStateException("open-chat kick target is null");
        Object listener = KakaoCallbacks.completion(loader, kick.getParameterTypes()[4], commandId);
        // No foreground UI and no abuse report. Authorization remains with KakaoTalk.
        kick.invoke(foreground, room, member, false, false, listener);
    }
}
