package dev.noa.kakao;

import java.lang.reflect.Method;
import java.lang.reflect.Modifier;

/** Resolves and dispatches sending-log operations. */
final class KakaoSendingResolver {
    private KakaoSendingResolver() {}

    static void resend(Object room, Object entry, long commandId) throws Exception {
        if (room == null || entry == null) {
            throw new IllegalArgumentException("resend room and entry are required");
        }
        Object manager = KakaoSignatureResolver.objectFor("sending-log-manager");
        KakaoSignatureResolver.invokeOperation(
                "prepare-resend", manager, new Object[] {entry});
        Object companion = KakaoSignatureResolver.objectFor("sending-log-request");
        Class<?> modeClass = KakaoSignatureResolver.classFor("sending-log-mode");
        @SuppressWarnings({"unchecked", "rawtypes"})
        Object mode = Enum.valueOf((Class) modeClass, "Resend");

        Method send = null;
        for (Method method : companion.getClass().getDeclaredMethods()) {
            Class<?>[] parameters = method.getParameterTypes();
            if (Modifier.isStatic(method.getModifiers())
                    || method.getReturnType() != void.class
                    || parameters.length != 5
                    || !parameters[0].isInstance(room)
                    || !parameters[1].isInstance(entry)
                    || parameters[2] != modeClass
                    || !parameters[3].isInterface()
                    || parameters[4] != boolean.class) continue;
            if (send != null) {
                throw new NoSuchMethodException("ambiguous resend request signature");
            }
            send = method;
        }
        if (send == null) {
            throw new NoSuchMethodException("resend request signature was not found");
        }
        Class<?> listenerType = send.getParameterTypes()[3];
        Object listener = KakaoCallbacks.send(
                KakaoSignatureResolver.appLoader(), listenerType, commandId);
        send.setAccessible(true);
        send.invoke(companion, room, entry, mode, listener, false);
    }
}
