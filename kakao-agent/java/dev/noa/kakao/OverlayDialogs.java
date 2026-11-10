package dev.noa.kakao;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Proxy;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.Map;

/** Main-looper-only overlay dialogs; no Activity or foreground transition is used. */
public final class OverlayDialogs {
    private static final int OVERLAY_TYPE = 2038; // TYPE_APPLICATION_OVERLAY, API 26+
    private static final int HISTORY_LIMIT = 64;
    private static final LinkedHashMap<String, Entry> HISTORY = new LinkedHashMap<>();
    private static Entry active;

    private OverlayDialogs() {}

    public static String execute(String payload) {
        try {
            Class<?> looper = Class.forName("android.os.Looper");
            Object main = looper.getMethod("getMainLooper").invoke(null);
            if (main == null || looper.getMethod("myLooper").invoke(null) != main)
                throw new IllegalStateException("dialog operation requires the main looper");
            expire();
            Object command = Class.forName("org.json.JSONObject").getConstructor(String.class).newInstance(payload);
            String operation = (String) get(command, "operation");
            if ("get".equals(operation) || "dismiss".equals(operation)) {
                String id = (String) get(command, "dialogId");
                Entry entry = HISTORY.get(id);
                if (entry == null) return json(error("not-found", "Dialog result not found in this agent session"));
                if ("dismiss".equals(operation)) finish(entry, "dismissed", null);
                return json(entry.result());
            }
            Object application = Class.forName("android.app.ActivityThread")
                    .getMethod("currentApplication").invoke(null);
            if (application == null) throw new IllegalStateException("Android application is not ready");
            Class<?> context = Class.forName("android.content.Context");
            boolean permission = Boolean.TRUE.equals(Class.forName("android.provider.Settings")
                    .getMethod("canDrawOverlays", context).invoke(null, application));
            if ("status".equals(operation)) {
                Map<String, Object> result = ok();
                result.put("overlayPermission", permission);
                result.put("activeDialogId", active == null ? null : active.id);
                result.put("retainedResults", HISTORY.size());
                result.put("historyLimit", HISTORY_LIMIT);
                return json(result);
            }
            if (!"show".equals(operation)) return json(error("bad-request", "Unknown dialog operation"));
            if (!permission) return json(error("permission-denied", "Enable display over other apps for KakaoTalk"));
            String id = (String) get(command, "dialogId");
            if (HISTORY.containsKey(id)) return json(HISTORY.get(id).result());
            if (active != null) return json(error("dialog-active", "Another Noa dialog is already displayed: " + active.id));
            Object content = get(command, "content");
            Entry entry = new Entry(id);
            Class<?> handler = Class.forName("android.os.Handler");
            entry.handler = handler.getConstructor(looper).newInstance(main);
            int theme = Class.forName("android.R$style").getField("Theme_Material_Light_Dialog_Alert").getInt(null);
            Class<?> builderType = Class.forName("android.app.AlertDialog$Builder");
            Object builder = builderType.getConstructor(context, int.class).newInstance(application, theme);
            builderType.getMethod("setTitle", CharSequence.class).invoke(builder, "Noa · " + get(content, "title"));
            builderType.getMethod("setMessage", CharSequence.class).invoke(builder, (String) get(content, "message"));
            boolean cancelable = (Boolean) get(content, "cancelable");
            builderType.getMethod("setCancelable", boolean.class).invoke(builder, cancelable);
            Object buttons = get(content, "buttons");
            int count = ((Number) buttons.getClass().getMethod("length").invoke(buttons)).intValue();
            if (count < 1 || count > 3) throw new IllegalArgumentException("1-3 buttons are required");
            String[] setters = {"setPositiveButton", "setNegativeButton", "setNeutralButton"};
            Class<?> click = Class.forName("android.content.DialogInterface$OnClickListener");
            for (int i = 0; i < count; i++) {
                Object button = buttons.getClass().getMethod("getJSONObject", int.class).invoke(buttons, i);
                final String buttonId = (String) get(button, "id");
                Object listener = listener(click, () -> finishFromUi(entry, "selected", buttonId));
                builderType.getMethod(setters[i], CharSequence.class, click)
                        .invoke(builder, (String) get(button, "label"), listener);
            }
            entry.dialog = builderType.getMethod("create").invoke(builder);
            Object window = call(entry.dialog, "getWindow");
            if (window == null) throw new IllegalStateException("Dialog window is missing");
            window.getClass().getMethod("setType", int.class).invoke(window, OVERLAY_TYPE);
            entry.dialog.getClass().getMethod("setCanceledOnTouchOutside", boolean.class).invoke(entry.dialog, cancelable);
            Class<?> cancel = Class.forName("android.content.DialogInterface$OnCancelListener");
            entry.dialog.getClass().getMethod("setOnCancelListener", cancel)
                    .invoke(entry.dialog, listener(cancel, () -> finishFromUi(entry, "cancelled", null)));
            Class<?> dismiss = Class.forName("android.content.DialogInterface$OnDismissListener");
            entry.dialog.getClass().getMethod("setOnDismissListener", dismiss)
                    .invoke(entry.dialog, listener(dismiss, () -> finishFromUi(entry, "dismissed", null)));
            long seconds = ((Number) get(content, "timeoutSeconds")).longValue();
            if (seconds < 1 || seconds > 600) throw new IllegalArgumentException("invalid dialog timeout");
            entry.createdAt = System.currentTimeMillis();
            entry.expiresAt = entry.createdAt + seconds * 1000L;
            entry.timeout = () -> finishFromUi(entry, "expired", null);
            boolean posted = Boolean.TRUE.equals(handler.getMethod("postDelayed", Runnable.class, long.class)
                    .invoke(entry.handler, entry.timeout, seconds * 1000L));
            if (!posted) throw new IllegalStateException("Unable to schedule dialog timeout");
            try {
                call(entry.dialog, "show");
                if (!Boolean.TRUE.equals(call(entry.dialog, "isShowing")))
                    throw new IllegalStateException("Android did not attach the dialog window");
            } catch (Throwable error) {
                handler.getMethod("removeCallbacks", Runnable.class).invoke(entry.handler, entry.timeout);
                try { call(entry.dialog, "dismiss"); } catch (Throwable ignored) {}
                throw error;
            }
            while (HISTORY.size() >= HISTORY_LIMIT) {
                Iterator<String> ids = HISTORY.keySet().iterator();
                ids.next(); ids.remove();
            }
            HISTORY.put(id, entry);
            active = entry;
            return json(entry.result());
        } catch (Throwable failure) {
            while (failure instanceof InvocationTargetException && failure.getCause() != null)
                failure = failure.getCause();
            return json(error(failure instanceof SecurityException ? "permission-denied" : "display-unavailable",
                    failure.toString()));
        }
    }

    private static void expire() {
        if (active != null && System.currentTimeMillis() >= active.expiresAt) finish(active, "expired", null);
    }

    private static void finishFromUi(Entry entry, String state, String button) {
        try { finish(entry, state, button); }
        catch (Throwable error) {
            // UI callbacks must never propagate an exception into KakaoTalk's main loop.
            entry.lastError = error.toString();
        }
    }

    private static void finish(Entry entry, String state, String button) {
        if (!"shown".equals(entry.state) || entry.finishing) return;
        if (System.currentTimeMillis() >= entry.expiresAt) { state = "expired"; button = null; }
        // Dismiss first so a failed removal remains retryable instead of losing an active window.
        entry.finishing = true;
        try {
            call(entry.dialog, "dismiss");
        } catch (Exception error) { throw new IllegalStateException("Unable to close dialog", error); }
        finally { entry.finishing = false; }
        entry.lastError = null;
        entry.state = state;
        entry.button = button;
        entry.finishedAt = System.currentTimeMillis();
        if (active == entry) active = null;
        try {
            entry.handler.getClass().getMethod("removeCallbacks", Runnable.class).invoke(entry.handler, entry.timeout);
        } catch (Exception ignored) {}
        entry.dialog = null;
        entry.handler = null;
        entry.timeout = null;
    }

    private static Object listener(Class<?> type, Runnable action) {
        return Proxy.newProxyInstance(type.getClassLoader(), new Class<?>[]{type}, (proxy, method, args) -> {
            if (method.getDeclaringClass() == Object.class) {
                if ("toString".equals(method.getName())) return "NoaOverlayListener";
                if ("hashCode".equals(method.getName())) return System.identityHashCode(proxy);
                if ("equals".equals(method.getName())) return proxy == args[0];
            }
            action.run();
            return null;
        });
    }
    private static Object get(Object json, String key) throws Exception {
        return json.getClass().getMethod("get", String.class).invoke(json, key);
    }
    private static Object call(Object object, String method) throws Exception {
        return object.getClass().getMethod(method).invoke(object);
    }
    private static Map<String, Object> ok() {
        Map<String, Object> value = new LinkedHashMap<>(); value.put("ok", true); return value;
    }
    private static Map<String, Object> error(String code, String message) {
        Map<String, Object> value = new LinkedHashMap<>();
        value.put("ok", false); value.put("errorCode", code); value.put("error", message); return value;
    }
    private static String json(Map<String, Object> value) {
        try { return Class.forName("org.json.JSONObject").getConstructor(Map.class).newInstance(value).toString(); }
        catch (Exception error) { throw new IllegalStateException("Unable to encode dialog result", error); }
    }
    private static final class Entry {
        final String id;
        Object dialog, handler;
        Runnable timeout;
        String state = "shown", button, lastError;
        boolean finishing;
        long createdAt, expiresAt, finishedAt;
        Entry(String id) { this.id = id; }
        Map<String, Object> result() {
            Map<String, Object> value = ok();
            value.put("dialogId", id); value.put("state", state); value.put("buttonId", button);
            value.put("createdAt", createdAt); value.put("expiresAt", expiresAt);
            value.put("finishedAt", finishedAt == 0 ? null : finishedAt);
            value.put("lastError", lastError);
            return value;
        }
    }
}
