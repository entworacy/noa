package dev.noa.kakao;

import java.lang.reflect.Method;
import java.lang.reflect.Modifier;

/** Checks the installed app version and its sending-log RichText contract. */
final class RichTextSupport {
    private RichTextSupport() {}

    static void require(Object application, ClassLoader loader) throws Exception {
        Class<?> context = Class.forName("android.content.Context");
        Object manager = context.getMethod("getPackageManager").invoke(application);
        Object info = Class.forName("android.content.pm.PackageManager")
                .getMethod("getPackageInfo", String.class, int.class)
                .invoke(manager, "com.kakao.talk", 0);
        String version = (String) info.getClass().getField("versionName").get(info);
        requireVersion(version);
        Class<?> richText = Class.forName("com.kakao.talk.chat.richtext.RichText", false, loader);
        Class<?> sendingLog = Class.forName(
                "com.kakao.talk.manager.send.sending.ChatSendingLog", false, loader);
        for (Method method : sendingLog.getDeclaredMethods()) {
            if (!Modifier.isStatic(method.getModifiers())
                    && method.getParameterTypes().length == 0
                    && method.getReturnType() == richText) return;
        }
        throw new NoSuchMethodException("KakaoTalk RichText sending-log contract is unavailable");
    }

    static void requireVersion(String version) {
        try {
            String[] parts = version == null ? new String[0] : version.split("\\.");
            if (parts.length >= 2 && parts[0].matches("[0-9]+")
                    && parts[1].matches("[0-9]+")) {
                int major = Integer.parseInt(parts[0]);
                int minor = Integer.parseInt(parts[1]);
                if (major > 26 || (major == 26 && minor >= 9)) return;
            }
        } catch (NumberFormatException ignored) {
            // Unknown or malformed versions must fail closed as well.
        }
        throw new IllegalStateException(
                "RichText requires KakaoTalk 26.9 or later (installed=" + version + ")");
    }
}
