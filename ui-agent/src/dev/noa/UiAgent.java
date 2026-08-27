package dev.noa;

import com.android.uiautomator.testrunner.UiAutomatorTestCase;
import com.android.uiautomator.core.Configurator;
import com.android.uiautomator.core.UiDevice;
import com.android.uiautomator.core.UiObject;
import com.android.uiautomator.core.UiObjectNotFoundException;
import com.android.uiautomator.core.UiScrollable;
import com.android.uiautomator.core.UiSelector;

import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.lang.reflect.Method;
import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.ArrayDeque;
import java.util.Base64;
import java.util.List;
import java.util.Locale;
import java.util.regex.Pattern;
import android.content.ClipData;
import android.app.UiAutomation;
import android.graphics.Rect;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityWindowInfo;

public final class UiAgent extends UiAutomatorTestCase {
    private static final int PORT = 47123;
    private static final String DUMP_PATH = "/data/local/tmp/noa-accessibility.xml";
    private static final String MEMBER_LIST_ID = "com.kakao.talk:id/recycler_view";
    private static final String PROFILE_NAME_ID = "com.kakao.talk.openlink:id/name";
    private static final String CHAT_TITLE_ID =
        "com.kakao.talk:id/toolbar_default_title_text";
    private static final String OPEN_CHAT_MESSAGE_ID = "com.kakao.talk:id/txt_message";
    private static final String OPEN_CHAT_JOIN_ID =
        "com.kakao.talk.openlink:id/join_layout";
    private static final String OPEN_CHAT_COVER_TITLE_PATTERN =
        "com\\.kakao\\.talk\\.openlink:id/title(?:_res_.*)?";
    private static final String OPEN_CHAT_PROFILE_NAME_PATTERN =
        "com\\.kakao\\.talk\\.openlink:id/profile_name(?:_res_.*)?";
    private static final String SETTING_BUTTON_ID = "com.kakao.talk:id/setting_button";
    private static final String RESEND_INDICATOR_ID = "com.kakao.talk:id/resend_indicator";
    private static final String BUBBLE_ID = "com.kakao.talk:id/bubble_linearlayout";
    private static final String[] KICK_LABELS = {
        "대화상대 내보내기", "Send out participant"
    };
    private static final String[] RESEND_LABELS = {"재전송", "Re-send"};
    private static final String[] OPEN_PROFILE_MORE_LABELS = {"더보기", "More"};
    private static final String[] COPY_LABELS = {"링크 복사", "Copy Link"};
    private static final String[] EXPAND_MEMBER_LABELS = {"펼치기", "Expand"};
    private static final String[] PROFILE_BLOCK_LABELS = {"차단", "Block"};
    private static final String[] SETTINGS_LABELS = {"설정", "Settings"};
    private static final String[] LEAVE_CHATROOM_LABELS = {
        "채팅방 나가기", "Leave chatroom"
    };
    private static final String[] LEAVE_LABELS = {"나가기", "Leave"};
    private static final String[] REMOVE_LABELS = {"내보내기", "Remove"};
    private volatile boolean running = true;
    private int sdkInt;
    private String clipboardSentinel;

    public void testServe() throws Exception {
        sdkInt = readAndroidSdk();
        verifyRequiredAndroidApi(sdkInt);
        getUiDevice();
        Configurator.getInstance()
            .setWaitForIdleTimeout(100)
            .setWaitForSelectorTimeout(0)
            .setActionAcknowledgmentTimeout(500)
            .setScrollAcknowledgmentTimeout(100);
        try (ServerSocket server = new ServerSocket()) {
            server.setReuseAddress(true);
            server.bind(new InetSocketAddress(InetAddress.getByName("127.0.0.1"), PORT));
            while (running && !Thread.currentThread().isInterrupted()) {
                try (Socket socket = server.accept()) {
                    handle(socket);
                } catch (IOException ignored) {
                }
            }
        }
    }

    private void handle(Socket socket) throws IOException {
        socket.setSoTimeout(5000);
        BufferedReader reader = new BufferedReader(
            new InputStreamReader(socket.getInputStream(), StandardCharsets.UTF_8)
        );
        BufferedWriter writer = new BufferedWriter(
            new OutputStreamWriter(socket.getOutputStream(), StandardCharsets.UTF_8)
        );
        String command = reader.readLine();
        if ("PING".equals(command)) {
            respond(writer, "NOA_UI_32");
            return;
        }
        if ("API_STATUS".equals(command)) {
            respond(writer, "OK LEGACY_UIAUTOMATOR SDK=" + sdkInt);
            return;
        }
        if (command != null && command.startsWith("WAIT_OPEN_CHAT_DESTINATION ")) {
            try {
                long timeout = boundedTimeout(
                    command.substring("WAIT_OPEN_CHAT_DESTINATION ".length())
                );
                respond(writer, waitOpenChatDestination(timeout, true));
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("WAIT_OPEN_CHAT_PROFILE ")) {
            try {
                String[] parts = command.split(" ", 3);
                if (parts.length != 3) {
                    throw new IllegalArgumentException(
                        "WAIT_OPEN_CHAT_PROFILE requires timeout and profile"
                    );
                }
                long timeout = boundedTimeout(parts[1]);
                String profile = decode(parts[2]);
                respond(writer, waitOpenChatProfile(profile, timeout));
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("WAIT_OPEN_CHAT_ENTERED ")) {
            try {
                long timeout = boundedTimeout(
                    command.substring("WAIT_OPEN_CHAT_ENTERED ".length())
                );
                respond(writer, waitOpenChatDestination(timeout, false));
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("WAIT_RESOURCE_TEXT ")) {
            try {
                String[] parts = command.split(" ", 4);
                if (parts.length != 4) {
                    throw new IllegalArgumentException(
                        "WAIT_RESOURCE_TEXT requires timeout, resource id and text"
                    );
                }
                long timeout = boundedTimeout(parts[1]);
                respond(
                    writer,
                    waitForResourceTextFast(parts[2], parts[3], timeout)
                        ? "OK" : "NOT_FOUND"
                );
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("WAIT_RESOURCE ")) {
            try {
                String[] parts = command.split(" ", 3);
                if (parts.length != 3) {
                    throw new IllegalArgumentException(
                        "WAIT_RESOURCE requires timeout and resource id"
                    );
                }
                long timeout = boundedTimeout(parts[1]);
                respond(
                    writer,
                    waitForResourceFast(parts[2], timeout) ? "OK" : "NOT_FOUND"
                );
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("WAIT_TEXT ")) {
            try {
                String[] parts = command.split(" ", 3);
                if (parts.length != 3) {
                    throw new IllegalArgumentException("WAIT_TEXT requires timeout and text");
                }
                long timeout = Math.max(0, Math.min(10000, Long.parseLong(parts[1])));
                respond(writer, waitForText(parts[2], timeout) ? "OK" : "NOT_FOUND");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("SCROLL_CLICK_TEXT ")) {
            try {
                String text = command.substring("SCROLL_CLICK_TEXT ".length());
                respond(writer, scrollAndClickUniqueText(text));
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if ("EXPAND_MEMBER_LIST".equals(command)) {
            try {
                respond(writer, expandMemberList() ? "OK" : "NOT_FOUND");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("CLICK_KICK_PROFILE ")) {
            try {
                String nickname = command.substring("CLICK_KICK_PROFILE ".length());
                respond(writer, clickKickForProfile(nickname, 8000) ? "OK" : "NOT_FOUND");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("CLICK_RESEND_TARGET ")) {
            try {
                String[] parts = command.split(" ");
                if (parts.length < 2 || parts.length > 130) {
                    throw new IllegalArgumentException(
                        "CLICK_RESEND_TARGET requires timeout and at most 128 targets"
                    );
                }
                long timeout = boundedTimeout(parts[1]);
                String[] targets = new String[parts.length - 2];
                for (int index = 2; index < parts.length; index++) {
                    targets[index - 2] = decode(parts[index]);
                }
                respond(writer, waitClickResendTarget(targets, timeout));
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("CLICK_RESOURCE_AT ")) {
            try {
                String[] parts = command.split(" ", 6);
                if (parts.length != 6) {
                    throw new IllegalArgumentException(
                        "CLICK_RESOURCE_AT requires bounds and resource id"
                    );
                }
                int expectedX = (Integer.parseInt(parts[1]) + Integer.parseInt(parts[3])) / 2;
                int expectedY = (Integer.parseInt(parts[2]) + Integer.parseInt(parts[4])) / 2;
                UiObject target = nearestResource(parts[5], expectedX, expectedY);
                if (target == null) {
                    respond(writer, "NOT_FOUND");
                    return;
                }
                if (!clickObject(target)) {
                    respond(writer, "ERR click rejected");
                    return;
                }
                respond(writer, "OK");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if ("CLICK_RESEND_CONFIRM".equals(command)) {
            try {
                respond(writer, waitClickLabels(RESEND_LABELS, 8000) ? "OK" : "NOT_FOUND");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("CLICK_RESOURCE ")) {
            try {
                String resourceId = command.substring("CLICK_RESOURCE ".length());
                respond(writer, clickResourceFast(resourceId) ? "OK" : "NOT_FOUND");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if ("CLICK_OPEN_LINK_COPY".equals(command)) {
            try {
                respond(writer, waitClickLabels(COPY_LABELS, 8000) ? "OK" : "NOT_FOUND");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if ("WAIT_OPEN_PROFILE_MORE".equals(command)) {
            try {
                respond(
                    writer,
                    waitForLabelsFast(OPEN_PROFILE_MORE_LABELS, 12000)
                        ? "OK" : "NOT_FOUND"
                );
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if ("WAIT_MEMBER_PROFILE_SHARE".equals(command)) {
            try {
                respond(writer, waitMemberProfileShare(5000));
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if ("CLICK_OPEN_PROFILE_MORE".equals(command)) {
            try {
                respond(
                    writer,
                    waitClickLabels(OPEN_PROFILE_MORE_LABELS, 5000)
                        ? "OK" : "NOT_FOUND"
                );
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if ("PREPARE_CLIPBOARD".equals(command)) {
            try {
                prepareClipboard();
                respond(writer, "OK");
            } catch (Throwable error) {
                clipboardSentinel = null;
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("WAIT_CLIPBOARD_CHANGE ")) {
            try {
                long timeout = boundedTimeout(
                    command.substring("WAIT_CLIPBOARD_CHANGE ".length())
                );
                respond(writer, waitClipboardChange(timeout));
            } catch (Throwable error) {
                clipboardSentinel = null;
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if ("CLICK_SETTINGS".equals(command)) {
            try {
                respond(writer, waitClickLabels(SETTINGS_LABELS, 5000) ? "OK" : "NOT_FOUND");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if ("SCROLL_CLICK_LEAVE_CHATROOM".equals(command)) {
            try {
                respond(
                    writer,
                    scrollClickResourceLabels(
                        SETTING_BUTTON_ID,
                        LEAVE_CHATROOM_LABELS,
                        12000
                    ) ? "OK" : "NOT_FOUND"
                );
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if ("CLICK_LEAVE_CONFIRM".equals(command)) {
            try {
                respond(writer, waitClickLabels(LEAVE_LABELS, 8000) ? "OK" : "NOT_FOUND");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if ("CLICK_KICK_CONFIRM".equals(command)) {
            try {
                respond(writer, waitClickLabels(REMOVE_LABELS, 8000) ? "OK" : "NOT_FOUND");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("WAIT_RESOURCE_GONE ")) {
            try {
                String[] parts = command.split(" ", 3);
                if (parts.length != 3) {
                    throw new IllegalArgumentException(
                        "WAIT_RESOURCE_GONE requires timeout and resource id"
                    );
                }
                long timeout = boundedTimeout(parts[1]);
                UiObject target = new UiObject(new UiSelector().resourceId(parts[2]));
                respond(writer, waitForGone(target, timeout) ? "OK" : "NOT_FOUND");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if ("WAIT_IDLE".equals(command)) {
            try {
                getUiDevice().waitForIdle(2500);
                respond(writer, "OK");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if ("DUMP".equals(command)) {
            try {
                getUiDevice().setCompressedLayoutHeirarchy(true);
                getUiDevice().dumpWindowHierarchy(DUMP_PATH);
                respond(writer, "OK");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("CLICK ")) {
            try {
                String[] coordinates = command.split(" ");
                if (coordinates.length != 3) {
                    throw new IllegalArgumentException("CLICK requires x and y");
                }
                int x = Integer.parseInt(coordinates[1]);
                int y = Integer.parseInt(coordinates[2]);
                respond(writer, getUiDevice().click(x, y) ? "OK" : "ERR click rejected");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("CLICK_LABEL ")) {
            try {
                String label = command.substring("CLICK_LABEL ".length());
                UiObject target = new UiObject(new UiSelector().description(label));
                if (!target.exists()) {
                    target = new UiObject(new UiSelector().text(label));
                }
                if (!target.exists()) {
                    respond(writer, "ERR label not found");
                    return;
                }
                if (!clickObject(target)) {
                    respond(writer, "ERR click rejected");
                    return;
                }
                respond(writer, "OK");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if (command != null && command.startsWith("CLICK_TEXT_AT ")) {
            try {
                String[] parts = command.split(" ", 6);
                if (parts.length != 6) {
                    throw new IllegalArgumentException(
                        "CLICK_TEXT_AT requires bounds and text"
                    );
                }
                int expectedX = (Integer.parseInt(parts[1]) + Integer.parseInt(parts[3])) / 2;
                int expectedY = (Integer.parseInt(parts[2]) + Integer.parseInt(parts[4])) / 2;
                String text = parts[5];
                UiObject nearest = null;
                long nearestDistance = Long.MAX_VALUE;
                for (int instance = 0; instance < 100; instance++) {
                    UiObject candidate = new UiObject(
                        new UiSelector().text(text).instance(instance)
                    );
                    if (!candidate.exists()) {
                        break;
                    }
                    Rect bounds = candidate.getBounds();
                    long dx = ((long) bounds.left + bounds.right) / 2 - expectedX;
                    long dy = ((long) bounds.top + bounds.bottom) / 2 - expectedY;
                    long distance = dx * dx + dy * dy;
                    if (distance < nearestDistance) {
                        nearest = candidate;
                        nearestDistance = distance;
                    }
                }
                if (nearest == null) {
                    respond(writer, "ERR text not found");
                    return;
                }
                if (!clickObject(nearest)) {
                    respond(writer, "ERR click rejected");
                    return;
                }
                respond(writer, "OK");
            } catch (Throwable error) {
                respond(writer, "ERR " + singleLine(error.toString()));
            }
            return;
        }
        if ("STOP".equals(command)) {
            running = false;
            respond(writer, "OK");
            return;
        }
        respond(writer, "ERR unsupported command");
    }

    private static void respond(BufferedWriter writer, String value) throws IOException {
        writer.write(value);
        writer.newLine();
        writer.flush();
    }

    private static String singleLine(String value) {
        return value.replace('\n', ' ').replace('\r', ' ');
    }

    private boolean waitForText(String text, long timeoutMs) throws InterruptedException {
        return waitForObject(new UiObject(new UiSelector().text(text)), timeoutMs);
    }

    private AccessibilityNodeInfo rootNode() throws Exception {
        Field field = UiDevice.class.getDeclaredField("mUiAutomationBridge");
        field.setAccessible(true);
        Object bridge = field.get(getUiDevice());
        if (bridge == null) {
            throw new IllegalStateException("UiAutomation bridge unavailable");
        }
        Class<?> bridgeClass = Class.forName(
            "com.android.uiautomator.core.UiAutomatorBridge"
        );
        Field automationField = bridgeClass.getDeclaredField("mUiAutomation");
        automationField.setAccessible(true);
        UiAutomation automation = (UiAutomation) automationField.get(bridge);
        List<AccessibilityWindowInfo> windows = automation.getWindows();
        AccessibilityNodeInfo active = null;
        for (AccessibilityWindowInfo window : windows) {
            try {
                if (window.isFocused()) {
                    AccessibilityNodeInfo root = window.getRoot();
                    if (root != null) {
                        return root;
                    }
                }
                if (active == null && window.isActive()) {
                    active = window.getRoot();
                }
            } finally {
                window.recycle();
            }
        }
        return active != null ? active : automation.getRootInActiveWindow();
    }

    private boolean waitForResourceFast(String resourceId, long timeoutMs)
            throws Exception {
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        while (true) {
            AccessibilityNodeInfo root = rootNode();
            if (root != null) {
                try {
                    if (!root.findAccessibilityNodeInfosByViewId(resourceId).isEmpty()) {
                        return true;
                    }
                } finally {
                    root.recycle();
                }
            }
            if (System.nanoTime() >= deadline) {
                return false;
            }
            Thread.sleep(25);
        }
    }

    private boolean waitForResourceTextFast(
        String resourceId,
        String text,
        long timeoutMs
    ) throws Exception {
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        while (true) {
            AccessibilityNodeInfo root = rootNode();
            if (root != null) {
                try {
                    for (AccessibilityNodeInfo node
                            : root.findAccessibilityNodeInfosByViewId(resourceId)) {
                        if (nodeMatchesLabel(node, new String[] {text})) {
                            return true;
                        }
                    }
                } finally {
                    root.recycle();
                }
            }
            if (System.nanoTime() >= deadline) {
                return false;
            }
            Thread.sleep(25);
        }
    }

    private boolean clickResourceFast(String resourceId) throws Exception {
        AccessibilityNodeInfo root = rootNode();
        if (root == null) {
            return false;
        }
        try {
            List<AccessibilityNodeInfo> nodes = root.findAccessibilityNodeInfosByViewId(
                resourceId
            );
            if (nodes.isEmpty()) {
                return false;
            }
            return clickAccessibilityNode(nodes.get(0));
        } finally {
            root.recycle();
        }
    }

    private boolean clickFirstLabelFast(String[] labels) throws Exception {
        AccessibilityNodeInfo root = rootNode();
        if (root == null) {
            return false;
        }
        ArrayDeque<AccessibilityNodeInfo> pending = new ArrayDeque<AccessibilityNodeInfo>();
        List<AccessibilityNodeInfo> visited = new ArrayList<AccessibilityNodeInfo>();
        pending.add(root);
        try {
            while (!pending.isEmpty()) {
                AccessibilityNodeInfo node = pending.removeFirst();
                visited.add(node);
                if (nodeMatchesLabel(node, labels)) {
                    return clickAccessibilityNode(node);
                }
                for (int index = 0; index < node.getChildCount(); index++) {
                    AccessibilityNodeInfo child = node.getChild(index);
                    if (child != null) {
                        pending.addLast(child);
                    }
                }
            }
            return false;
        } finally {
            for (int index = visited.size() - 1; index >= 0; index--) {
                visited.get(index).recycle();
            }
            while (!pending.isEmpty()) {
                pending.removeFirst().recycle();
            }
        }
    }

    private boolean waitForLabelsFast(String[] labels, long timeoutMs) throws Exception {
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        while (true) {
            AccessibilityNodeInfo root = rootNode();
            if (root != null) {
                ArrayDeque<AccessibilityNodeInfo> pending =
                    new ArrayDeque<AccessibilityNodeInfo>();
                List<AccessibilityNodeInfo> visited =
                    new ArrayList<AccessibilityNodeInfo>();
                pending.add(root);
                try {
                    while (!pending.isEmpty()) {
                        AccessibilityNodeInfo node = pending.removeFirst();
                        visited.add(node);
                        if (nodeMatchesLabel(node, labels)) {
                            return true;
                        }
                        for (int index = 0; index < node.getChildCount(); index++) {
                            AccessibilityNodeInfo child = node.getChild(index);
                            if (child != null) {
                                pending.addLast(child);
                            }
                        }
                    }
                } finally {
                    for (int index = visited.size() - 1; index >= 0; index--) {
                        visited.get(index).recycle();
                    }
                    while (!pending.isEmpty()) {
                        pending.removeFirst().recycle();
                    }
                }
            }
            if (System.nanoTime() >= deadline) {
                return false;
            }
            Thread.sleep(25);
        }
    }

    private String waitMemberProfileShare(long timeoutMs) throws Exception {
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        long blockSeenAt = 0;
        while (true) {
            if (firstLabel(OPEN_PROFILE_MORE_LABELS) != null) {
                return "SHAREABLE";
            }
            if (firstLabel(PROFILE_BLOCK_LABELS) != null) {
                if (blockSeenAt == 0) {
                    blockSeenAt = System.nanoTime();
                } else if (System.nanoTime() - blockSeenAt >= 750000000L) {
                    return "NOT_SHAREABLE";
                }
            }
            if (System.nanoTime() >= deadline) {
                return "NOT_FOUND";
            }
            Thread.sleep(25);
        }
    }

    private static boolean nodeMatchesLabel(
        AccessibilityNodeInfo node,
        String[] labels
    ) {
        CharSequence text = node.getText();
        CharSequence description = node.getContentDescription();
        for (String label : labels) {
            if ((text != null && label.contentEquals(text))
                    || (description != null && label.contentEquals(description))) {
                return true;
            }
        }
        return false;
    }

    private boolean clickAccessibilityNode(AccessibilityNodeInfo node) {
        AccessibilityNodeInfo target = node;
        while (target != null) {
            if (target.isClickable()
                    && target.performAction(AccessibilityNodeInfo.ACTION_CLICK)) {
                return true;
            }
            target = target.getParent();
        }
        Rect bounds = new Rect();
        node.getBoundsInScreen(bounds);
        return !bounds.isEmpty() && getUiDevice().click(bounds.centerX(), bounds.centerY());
    }

    private long boundedTimeout(String value) {
        return Math.max(0, Math.min(20000, Long.parseLong(value)));
    }

    private boolean waitForObject(UiObject target, long timeoutMs) throws InterruptedException {
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        while (true) {
            if (target.exists()) {
                return true;
            }
            if (System.nanoTime() >= deadline) {
                return false;
            }
            Thread.sleep(25);
        }
    }

    private boolean waitForGone(UiObject target, long timeoutMs) throws InterruptedException {
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        while (true) {
            if (!target.exists()) {
                return true;
            }
            if (System.nanoTime() >= deadline) {
                return false;
            }
            Thread.sleep(25);
        }
    }

    private String waitOpenChatDestination(long timeoutMs, boolean includeCover)
            throws Exception {
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        while (true) {
            UiObject rejection = new UiObject(
                new UiSelector().resourceId(OPEN_CHAT_MESSAGE_ID)
            );
            if (rejection.exists()) {
                String message = objectText(rejection);
                if (!message.isEmpty()) {
                    return "REJECTED " + encode(message);
                }
            }
            UiObject entered = new UiObject(new UiSelector().resourceId(CHAT_TITLE_ID));
            if (entered.exists()) {
                String title = objectText(entered);
                if (!title.isEmpty()) {
                    return "ENTERED " + encode(title);
                }
            }
            if (includeCover) {
                UiObject join = new UiObject(new UiSelector().resourceId(OPEN_CHAT_JOIN_ID));
                UiObject cover = new UiObject(
                    new UiSelector().resourceIdMatches(OPEN_CHAT_COVER_TITLE_PATTERN)
                );
                if (join.exists() && cover.exists()) {
                    String title = objectText(cover);
                    if (!title.isEmpty()) {
                        return "COVER " + encode(title);
                    }
                }
            }
            if (System.nanoTime() >= deadline) {
                return "NOT_FOUND";
            }
            Thread.sleep(25);
        }
    }

    private String waitOpenChatProfile(String profile, long timeoutMs) throws Exception {
        if (profile.isEmpty()) {
            return "ERR empty profile";
        }
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        while (true) {
            UiObject rejection = new UiObject(
                new UiSelector().resourceId(OPEN_CHAT_MESSAGE_ID)
            );
            if (rejection.exists()) {
                String message = objectText(rejection);
                if (!message.isEmpty()) {
                    return "REJECTED " + encode(message);
                }
            }
            UiObject entered = new UiObject(new UiSelector().resourceId(CHAT_TITLE_ID));
            if (entered.exists()) {
                String title = objectText(entered);
                if (!title.isEmpty()) {
                    return "ENTERED " + encode(title);
                }
            }
            UiObject selected = null;
            int matches = 0;
            for (int instance = 0; instance < 100; instance++) {
                UiObject candidate = new UiObject(
                    new UiSelector()
                        .resourceIdMatches(OPEN_CHAT_PROFILE_NAME_PATTERN)
                        .text(profile)
                        .instance(instance)
                );
                if (!candidate.exists()) {
                    break;
                }
                selected = candidate;
                matches++;
            }
            if (matches > 1) {
                return "AMBIGUOUS";
            }
            if (selected != null) {
                if (!clickObject(selected)) {
                    return "ERR profile click rejected";
                }
                return "PROFILE " + encode(profile);
            }
            if (System.nanoTime() >= deadline) {
                return "NOT_FOUND";
            }
            Thread.sleep(25);
        }
    }

    private static String objectText(UiObject object) throws Exception {
        String text = object.getText();
        if (text == null || text.trim().isEmpty()) {
            text = object.getContentDescription();
        }
        return text == null ? "" : text.trim();
    }

    private static String encode(String value) {
        return Base64.getEncoder().withoutPadding().encodeToString(
            value.getBytes(StandardCharsets.UTF_8)
        );
    }

    private static String decode(String value) {
        return new String(Base64.getDecoder().decode(value), StandardCharsets.UTF_8);
    }

    private boolean waitForResourceText(
        String resourceId,
        String text,
        long timeoutMs
    ) throws InterruptedException {
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        UiObject byText = new UiObject(new UiSelector().resourceId(resourceId).text(text));
        UiObject byDescription = new UiObject(
            new UiSelector().resourceId(resourceId).description(text)
        );
        while (true) {
            if (byText.exists() || byDescription.exists()) {
                return true;
            }
            if (System.nanoTime() >= deadline) {
                return false;
            }
            Thread.sleep(25);
        }
    }

    private UiObject nearestResource(String resourceId, int expectedX, int expectedY)
            throws Exception {
        UiObject nearest = null;
        long nearestDistance = Long.MAX_VALUE;
        for (int instance = 0; instance < 100; instance++) {
            UiObject candidate = new UiObject(
                new UiSelector().resourceId(resourceId).instance(instance)
            );
            if (!candidate.exists()) {
                break;
            }
            Rect bounds = candidate.getBounds();
            long dx = ((long) bounds.left + bounds.right) / 2 - expectedX;
            long dy = ((long) bounds.top + bounds.bottom) / 2 - expectedY;
            long distance = dx * dx + dy * dy;
            if (distance < nearestDistance) {
                nearest = candidate;
                nearestDistance = distance;
            }
        }
        return nearest;
    }

    private boolean clickObject(UiObject target) throws Exception {
        final Rect bounds;
        try {
            bounds = target.getBounds();
        } catch (UiObjectNotFoundException ignored) {
            return false;
        }
        if (!getUiDevice().click(
            (bounds.left + bounds.right) / 2,
            (bounds.top + bounds.bottom) / 2
        )) {
            return false;
        }
        getUiDevice().waitForIdle(250);
        return true;
    }

    private boolean waitClickLabels(String[] labels, long timeoutMs) throws Exception {
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        while (true) {
            if (clickFirstLabelFast(labels)) {
                return true;
            }
            if (System.nanoTime() >= deadline) {
                return false;
            }
            Thread.sleep(25);
        }
    }

    private Object clipboardService() throws Exception {
        Class<?> serviceManager = Class.forName("android.os.ServiceManager");
        Object binder = serviceManager
            .getMethod("getService", String.class)
            .invoke(null, "clipboard");
        if (binder == null) {
            throw new IllegalStateException("clipboard service unavailable");
        }
        Class<?> clipboardStub = Class.forName("android.content.IClipboard$Stub");
        Object service = clipboardStub
            .getMethod("asInterface", Class.forName("android.os.IBinder"))
            .invoke(null, binder);
        if (service == null) {
            throw new IllegalStateException("clipboard interface unavailable");
        }
        return service;
    }

    private int clipboardUserId() throws Exception {
        return ((Integer) Class.forName("android.os.UserHandle")
            .getMethod("myUserId")
            .invoke(null)).intValue();
    }

    private void prepareClipboard() throws Exception {
        Object service = clipboardService();
        Class<?> clipboard = Class.forName("android.content.IClipboard");
        clipboard.getMethod(
            "clearPrimaryClip",
            String.class,
            String.class,
            Integer.TYPE,
            Integer.TYPE
        ).invoke(
            service,
            "com.android.shell",
            null,
            clipboardUserId(),
            0
        );
        String current = readClipboard(service);
        if (current != null) {
            throw new IllegalStateException("clipboard clear verification failed");
        }
        clipboardSentinel = "";
    }

    private String waitClipboardChange(long timeoutMs) throws Exception {
        String sentinel = clipboardSentinel;
        if (sentinel == null) {
            throw new IllegalStateException("clipboard was not prepared");
        }
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        try {
            Object service = clipboardService();
            while (true) {
                String value = readClipboard(service);
                if (value != null && !value.isEmpty() && !sentinel.equals(value)) {
                    return "VALUE " + encode(value);
                }
                if (System.nanoTime() >= deadline) {
                    return "NOT_FOUND";
                }
                Thread.sleep(25);
            }
        } finally {
            clipboardSentinel = null;
        }
    }

    private String readClipboard(Object service) throws Exception {
        Class<?> clipboard = Class.forName("android.content.IClipboard");
        ClipData data = (ClipData) clipboard.getMethod(
            "getPrimaryClip",
            String.class,
            String.class,
            Integer.TYPE,
            Integer.TYPE
        ).invoke(service, "com.android.shell", null, clipboardUserId(), 0);
        if (data == null || data.getItemCount() == 0) {
            return null;
        }
        CharSequence value = data.getItemAt(0).getText();
        return value == null ? null : value.toString().trim();
    }

    private boolean scrollClickResourceLabels(
        String resourceId,
        String[] labels,
        long timeoutMs
    ) throws Exception {
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        UiScrollable list = new UiScrollable(
            new UiSelector().resourceId(MEMBER_LIST_ID).scrollable(true)
        );
        while (true) {
            UiObject target = firstResourceLabel(resourceId, labels);
            if (target != null) {
                return clickObject(target);
            }
            if (list.exists()) {
                list = list.setAsVerticalList();
                if (!list.scrollForward(20)) {
                    return false;
                }
            }
            if (System.nanoTime() >= deadline) {
                return false;
            }
            Thread.sleep(25);
        }
    }

    private String scrollAndClickUniqueText(String text) throws Exception {
        if (text.isEmpty()) {
            return "ERR empty text";
        }
        // ChatRoomSideActivity is drawn over the chat room. UiObject/UiScrollable
        // searches may bind to the covered chat window and open another sender's
        // profile, so both lookup and scrolling must stay inside the focused window.
        Rect viewport = focusedScrollableBounds();
        if (viewport == null) {
            return "ERR focused scrollable member list not found";
        }
        for (int swipe = 0; swipe < 6; swipe++) {
            swipeVertical(viewport, false);
        }
        for (int swipe = 0; swipe <= 120; swipe++) {
            String result = clickUniqueFocusedText(text, viewport);
            if (!"NOT_FOUND".equals(result)) {
                return result;
            }
            Rect currentViewport = focusedScrollableBounds();
            if (currentViewport == null) {
                return "ERR focused scrollable member list disappeared";
            }
            viewport = currentViewport;
            swipeVertical(viewport, true);
        }
        return "NOT_FOUND";
    }

    private boolean expandMemberList() throws Exception {
        Rect viewport = focusedScrollableBounds();
        if (viewport == null) {
            return false;
        }
        for (int swipe = 0; swipe <= 40; swipe++) {
            if (clickFirstLabelFast(EXPAND_MEMBER_LABELS)) {
                return true;
            }
            Rect currentViewport = focusedScrollableBounds();
            if (currentViewport == null) {
                return false;
            }
            viewport = currentViewport;
            swipeVertical(viewport, true);
        }
        return false;
    }

    private Rect focusedScrollableBounds() throws Exception {
        AccessibilityNodeInfo root = rootNode();
        if (root == null) {
            return null;
        }
        ArrayDeque<AccessibilityNodeInfo> pending =
            new ArrayDeque<AccessibilityNodeInfo>();
        List<AccessibilityNodeInfo> visited =
            new ArrayList<AccessibilityNodeInfo>();
        Rect largest = null;
        long largestArea = 0;
        pending.add(root);
        try {
            while (!pending.isEmpty()) {
                AccessibilityNodeInfo node = pending.removeFirst();
                visited.add(node);
                if (node.isScrollable()) {
                    Rect bounds = new Rect();
                    node.getBoundsInScreen(bounds);
                    long area = (long) bounds.width() * bounds.height();
                    if (!bounds.isEmpty() && area > largestArea) {
                        largest = new Rect(bounds);
                        largestArea = area;
                    }
                }
                for (int index = 0; index < node.getChildCount(); index++) {
                    AccessibilityNodeInfo child = node.getChild(index);
                    if (child != null) {
                        pending.addLast(child);
                    }
                }
            }
            return largest;
        } finally {
            recycleNodes(visited, pending);
        }
    }

    private String clickUniqueFocusedText(String text, Rect viewport) throws Exception {
        AccessibilityNodeInfo root = rootNode();
        if (root == null) {
            return "NOT_FOUND";
        }
        ArrayDeque<AccessibilityNodeInfo> pending =
            new ArrayDeque<AccessibilityNodeInfo>();
        List<AccessibilityNodeInfo> visited =
            new ArrayList<AccessibilityNodeInfo>();
        List<AccessibilityNodeInfo> candidates =
            new ArrayList<AccessibilityNodeInfo>();
        List<Rect> candidateBounds = new ArrayList<Rect>();
        pending.add(root);
        try {
            while (!pending.isEmpty()) {
                AccessibilityNodeInfo node = pending.removeFirst();
                visited.add(node);
                CharSequence value = node.getText();
                if (value != null && text.contentEquals(value)) {
                    Rect bounds = new Rect();
                    node.getBoundsInScreen(bounds);
                    if (!bounds.isEmpty()
                            && viewport.contains(bounds.centerX(), bounds.centerY())
                            && !candidateBounds.contains(bounds)) {
                        candidates.add(node);
                        candidateBounds.add(bounds);
                    }
                }
                for (int index = 0; index < node.getChildCount(); index++) {
                    AccessibilityNodeInfo child = node.getChild(index);
                    if (child != null) {
                        pending.addLast(child);
                    }
                }
            }
            if (candidates.size() > 1) {
                return "AMBIGUOUS";
            }
            if (candidates.size() == 1) {
                return clickAccessibilityNode(candidates.get(0))
                    ? "OK" : "ERR click rejected";
            }
            return "NOT_FOUND";
        } finally {
            recycleNodes(visited, pending);
        }
    }

    private static void recycleNodes(
        List<AccessibilityNodeInfo> visited,
        ArrayDeque<AccessibilityNodeInfo> pending
    ) {
        for (int index = visited.size() - 1; index >= 0; index--) {
            visited.get(index).recycle();
        }
        while (!pending.isEmpty()) {
            pending.removeFirst().recycle();
        }
    }

    private boolean swipeVertical(Rect bounds, boolean forward) throws Exception {
        int width = bounds.right - bounds.left;
        int height = bounds.bottom - bounds.top;
        if (width <= 0 || height <= 0) {
            return false;
        }
        int x = bounds.left + width / 2;
        int upper = bounds.top + height / 3;
        int lower = bounds.bottom - height / 8;
        return forward
            ? getUiDevice().swipe(x, lower, x, upper, 5)
            : getUiDevice().swipe(x, upper, x, lower, 5);
    }

    private boolean clickKickForProfile(String nickname, long timeoutMs) throws Exception {
        if (nickname.isEmpty()) {
            return false;
        }
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        do {
            UiObject profile = new UiObject(
                new UiSelector().resourceId(PROFILE_NAME_ID).text(nickname)
            );
            if (profile.exists()) {
                UiObject action = firstLabel(KICK_LABELS);
                if (action != null) {
                    if (!clickObject(action)) {
                        throw new IllegalStateException("kick click rejected");
                    }
                    return true;
                }
            }
            Thread.sleep(25);
        } while (System.nanoTime() < deadline);
        return false;
    }

    private String waitClickResendTarget(String[] targets, long timeoutMs) throws Exception {
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        String lastResult = "NOT_FOUND";
        while (true) {
            ResendCandidate candidate = bestResendCandidate(targets);
            if (candidate != null) {
                if (!clickObject(candidate.indicator)) {
                    throw new IllegalStateException("resend indicator click rejected");
                }
                return "OK";
            }
            if (targets.length == 0 && visibleObjects(RESEND_INDICATOR_ID).size() > 1) {
                lastResult = "AMBIGUOUS";
            }
            if (System.nanoTime() >= deadline) {
                return lastResult;
            }
            Thread.sleep(25);
        }
    }

    private ResendCandidate bestResendCandidate(String[] targets) throws Exception {
        List<UiObject> indicators = visibleObjects(RESEND_INDICATOR_ID);
        if (indicators.isEmpty()) {
            return null;
        }
        if (targets.length == 0 && indicators.size() != 1) {
            return null;
        }
        List<Rect> bubbles = visibleBounds(BUBBLE_ID);
        ResendCandidate best = null;
        for (UiObject indicator : indicators) {
            Rect indicatorBounds = indicator.getBounds();
            Rect bubble = smallestContainer(bubbles, indicatorBounds);
            if (bubble == null) {
                continue;
            }
            int score = targets.length == 0 ? 1 : scoreBubble(bubble, targets);
            if (score == 0) {
                continue;
            }
            if (best == null || score > best.score
                    || (score == best.score && indicatorBounds.bottom > best.bounds.bottom)) {
                best = new ResendCandidate(indicator, indicatorBounds, score);
            }
        }
        return best;
    }

    private int scoreBubble(Rect bubble, String[] targets) throws Exception {
        int score = 0;
        for (String target : targets) {
            if (target.isEmpty()) {
                continue;
            }
            String regex = "(?is).*" + Pattern.quote(target) + ".*";
            score += scoreMatches(
                new UiSelector().textMatches(regex),
                bubble,
                target,
                false
            );
            score += scoreMatches(
                new UiSelector().descriptionMatches(regex),
                bubble,
                target,
                true
            );
        }
        return score;
    }

    private int scoreMatches(
        UiSelector selector,
        Rect bubble,
        String target,
        boolean description
    ) throws Exception {
        int score = 0;
        for (int instance = 0; instance < 100; instance++) {
            UiObject candidate = new UiObject(selector.instance(instance));
            if (!candidate.exists()) {
                break;
            }
            if (!contains(bubble, candidate.getBounds())) {
                continue;
            }
            String value = description
                ? candidate.getContentDescription()
                : candidate.getText();
            score += matchScore(value, target);
        }
        return score;
    }

    private List<UiObject> visibleObjects(String resourceId) {
        List<UiObject> objects = new ArrayList<UiObject>();
        for (int instance = 0; instance < 100; instance++) {
            UiObject object = new UiObject(
                new UiSelector().resourceId(resourceId).instance(instance)
            );
            if (!object.exists()) {
                break;
            }
            objects.add(object);
        }
        return objects;
    }

    private List<Rect> visibleBounds(String resourceId) throws Exception {
        List<Rect> bounds = new ArrayList<Rect>();
        for (UiObject object : visibleObjects(resourceId)) {
            bounds.add(object.getBounds());
        }
        return bounds;
    }

    private static Rect smallestContainer(List<Rect> containers, Rect inner) {
        Rect smallest = null;
        long smallestArea = Long.MAX_VALUE;
        for (Rect container : containers) {
            if (!contains(container, inner)) {
                continue;
            }
            long area = (long) (container.right - container.left)
                * (container.bottom - container.top);
            if (area < smallestArea) {
                smallest = container;
                smallestArea = area;
            }
        }
        return smallest;
    }

    private static boolean contains(Rect outer, Rect inner) {
        return outer.left <= inner.left && outer.top <= inner.top
            && outer.right >= inner.right && outer.bottom >= inner.bottom;
    }

    private static int matchScore(String value, String target) {
        if (value == null) {
            return 0;
        }
        String normalizedValue = value.trim().toLowerCase(Locale.ROOT);
        String normalizedTarget = target.trim().toLowerCase(Locale.ROOT);
        if (normalizedValue.isEmpty() || normalizedTarget.isEmpty()) {
            return 0;
        }
        if (normalizedValue.equals(normalizedTarget)) {
            return 1000 + normalizedTarget.codePointCount(0, normalizedTarget.length());
        }
        if (normalizedValue.contains(normalizedTarget)
                || normalizedTarget.contains(normalizedValue)) {
            return Math.min(
                normalizedValue.codePointCount(0, normalizedValue.length()),
                normalizedTarget.codePointCount(0, normalizedTarget.length())
            );
        }
        return 0;
    }

    private static final class ResendCandidate {
        private final UiObject indicator;
        private final Rect bounds;
        private final int score;

        private ResendCandidate(UiObject indicator, Rect bounds, int score) {
            this.indicator = indicator;
            this.bounds = bounds;
            this.score = score;
        }
    }

    private UiObject firstLabel(String[] labels) {
        for (String label : labels) {
            UiObject target = new UiObject(new UiSelector().description(label));
            if (target.exists()) {
                return target;
            }
            target = new UiObject(new UiSelector().text(label));
            if (target.exists()) {
                return target;
            }
        }
        return null;
    }

    private UiObject firstResourceLabel(String resourceId, String[] labels) {
        for (String label : labels) {
            UiObject target = new UiObject(
                new UiSelector().resourceId(resourceId).description(label)
            );
            if (target.exists()) {
                return target;
            }
            target = new UiObject(
                new UiSelector().resourceId(resourceId).text(label)
            );
            if (target.exists()) {
                return target;
            }
        }
        return null;
    }

    private static void verifyRequiredAndroidApi(int sdkInt) throws Exception {
        if (sdkInt < 26) {
            throw new IllegalStateException("Android API 26 이상이 필요합니다: " + sdkInt);
        }
        requireConstructor(UiSelector.class);
        requireConstructor(UiObject.class, UiSelector.class);
        requireConstructor(UiScrollable.class, UiSelector.class);
        requireMethod(Configurator.class, "getInstance", Configurator.class);
        requireMethod(
            Configurator.class,
            "setWaitForIdleTimeout",
            Configurator.class,
            Long.TYPE
        );
        requireMethod(
            Configurator.class,
            "setWaitForSelectorTimeout",
            Configurator.class,
            Long.TYPE
        );
        requireMethod(
            Configurator.class,
            "setActionAcknowledgmentTimeout",
            Configurator.class,
            Long.TYPE
        );
        requireMethod(
            Configurator.class,
            "setScrollAcknowledgmentTimeout",
            Configurator.class,
            Long.TYPE
        );

        requireMethod(UiAutomatorTestCase.class, "getUiDevice", UiDevice.class);
        Field bridgeField = UiDevice.class.getDeclaredField("mUiAutomationBridge");
        bridgeField.setAccessible(true);
        Class<?> bridgeClass = Class.forName(
            "com.android.uiautomator.core.UiAutomatorBridge"
        );
        Field automationField = bridgeClass.getDeclaredField("mUiAutomation");
        automationField.setAccessible(true);
        requireMethod(
            bridgeClass,
            "getRootInActiveWindow",
            AccessibilityNodeInfo.class
        );
        requireMethod(UiAutomation.class, "getWindows", List.class);
        requireMethod(
            UiAutomation.class,
            "getRootInActiveWindow",
            AccessibilityNodeInfo.class
        );
        requireMethod(AccessibilityWindowInfo.class, "isActive", Boolean.TYPE);
        requireMethod(AccessibilityWindowInfo.class, "isFocused", Boolean.TYPE);
        requireMethod(
            AccessibilityWindowInfo.class,
            "getRoot",
            AccessibilityNodeInfo.class
        );
        requireMethod(AccessibilityWindowInfo.class, "recycle", Void.TYPE);
        requireMethod(
            AccessibilityNodeInfo.class,
            "findAccessibilityNodeInfosByViewId",
            List.class,
            String.class
        );
        requireMethod(AccessibilityNodeInfo.class, "getText", CharSequence.class);
        requireMethod(
            AccessibilityNodeInfo.class,
            "getContentDescription",
            CharSequence.class
        );
        requireMethod(AccessibilityNodeInfo.class, "getChildCount", Integer.TYPE);
        requireMethod(
            AccessibilityNodeInfo.class,
            "getChild",
            AccessibilityNodeInfo.class,
            Integer.TYPE
        );
        requireMethod(
            AccessibilityNodeInfo.class,
            "getParent",
            AccessibilityNodeInfo.class
        );
        requireMethod(AccessibilityNodeInfo.class, "isClickable", Boolean.TYPE);
        requireMethod(
            AccessibilityNodeInfo.class,
            "performAction",
            Boolean.TYPE,
            Integer.TYPE
        );
        requireMethod(
            AccessibilityNodeInfo.class,
            "getBoundsInScreen",
            Void.TYPE,
            Rect.class
        );
        requireMethod(AccessibilityNodeInfo.class, "recycle", Void.TYPE);
        requireMethod(ClipData.class, "getItemCount", Integer.TYPE);
        requireMethod(ClipData.class, "getItemAt", ClipData.Item.class, Integer.TYPE);
        requireMethod(ClipData.Item.class, "getText", CharSequence.class);

        Class<?> binder = Class.forName("android.os.IBinder");
        Class<?> serviceManager = Class.forName("android.os.ServiceManager");
        Class<?> clipboard = Class.forName("android.content.IClipboard");
        Class<?> clipboardStub = Class.forName("android.content.IClipboard$Stub");
        requireMethod(serviceManager, "getService", binder, String.class);
        requireMethod(clipboardStub, "asInterface", clipboard, binder);
        requireMethod(
            clipboard,
            "clearPrimaryClip",
            Void.TYPE,
            String.class,
            String.class,
            Integer.TYPE,
            Integer.TYPE
        );
        requireMethod(
            clipboard,
            "getPrimaryClip",
            ClipData.class,
            String.class,
            String.class,
            Integer.TYPE,
            Integer.TYPE
        );
        requireMethod(
            Class.forName("android.os.UserHandle"),
            "myUserId",
            Integer.TYPE
        );
        requireMethod(UiSelector.class, "description", UiSelector.class, String.class);
        requireMethod(UiSelector.class, "descriptionMatches", UiSelector.class, String.class);
        requireMethod(UiSelector.class, "text", UiSelector.class, String.class);
        requireMethod(UiSelector.class, "textMatches", UiSelector.class, String.class);
        requireMethod(UiSelector.class, "resourceId", UiSelector.class, String.class);
        requireMethod(UiSelector.class, "resourceIdMatches", UiSelector.class, String.class);
        requireMethod(UiSelector.class, "className", UiSelector.class, String.class);
        requireMethod(UiSelector.class, "scrollable", UiSelector.class, Boolean.TYPE);
        requireMethod(UiSelector.class, "instance", UiSelector.class, Integer.TYPE);

        requireMethod(UiObject.class, "exists", Boolean.TYPE);
        requireMethod(UiObject.class, "click", Boolean.TYPE);
        requireMethod(UiObject.class, "getBounds", Rect.class);
        requireMethod(UiObject.class, "getText", String.class);
        requireMethod(UiObject.class, "getContentDescription", String.class);

        requireMethod(UiScrollable.class, "setAsVerticalList", UiScrollable.class);
        requireMethod(
            UiScrollable.class,
            "scrollToBeginning",
            Boolean.TYPE,
            Integer.TYPE,
            Integer.TYPE
        );
        requireMethod(
            UiScrollable.class,
            "scrollForward",
            Boolean.TYPE,
            Integer.TYPE
        );

        requireMethod(UiDevice.class, "click", Boolean.TYPE, Integer.TYPE, Integer.TYPE);
        requireMethod(
            UiDevice.class,
            "swipe",
            Boolean.TYPE,
            Integer.TYPE,
            Integer.TYPE,
            Integer.TYPE,
            Integer.TYPE,
            Integer.TYPE
        );
        requireMethod(UiDevice.class, "waitForIdle", Void.TYPE, Long.TYPE);
        requireMethod(
            UiDevice.class,
            "setCompressedLayoutHeirarchy",
            Void.TYPE,
            Boolean.TYPE
        );
        requireMethod(UiDevice.class, "dumpWindowHierarchy", Void.TYPE, String.class);
    }

    private static int readAndroidSdk() throws Exception {
        return Class.forName("android.os.Build$VERSION")
            .getField("SDK_INT")
            .getInt(null);
    }

    private static void requireConstructor(Class<?> owner, Class<?>... parameters)
            throws Exception {
        owner.getConstructor(parameters);
    }

    private static void requireMethod(
        Class<?> owner,
        String name,
        Class<?> returnType,
        Class<?>... parameters
    ) throws Exception {
        Method method = owner.getMethod(name, parameters);
        if (!method.getReturnType().equals(returnType)) {
            throw new NoSuchMethodException(
                owner.getName() + "." + name + " return type: expected "
                    + returnType.getName() + ", actual "
                    + method.getReturnType().getName()
            );
        }
    }
}
