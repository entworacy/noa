package dev.noa;

import com.android.uiautomator.testrunner.UiAutomatorTestCase;
import com.android.uiautomator.core.UiObject;
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
import java.lang.reflect.Field;
import java.lang.reflect.Method;

public final class UiAgent extends UiAutomatorTestCase {
    private static final int PORT = 47123;
    private static final String DUMP_PATH = "noa/custom-accessibility.xml";
    private volatile boolean running = true;

    public void testServe() throws Exception {
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
            respond(writer, "NOA_UI_7");
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
                clearUiAutomationCache();
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
                if (!target.click()) {
                    respond(writer, "ERR click rejected");
                    return;
                }
                getUiDevice().waitForIdle(250);
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

    private void clearUiAutomationCache() {
        try {
            Object device = getUiDevice();
            Field bridgeField = device.getClass().getDeclaredField("mUiAutomationBridge");
            bridgeField.setAccessible(true);
            Object bridge = bridgeField.get(device);
            Field automationField = bridge.getClass().getSuperclass().getDeclaredField("mUiAutomation");
            automationField.setAccessible(true);
            Object automation = automationField.get(bridge);
            Method clearCache = automation.getClass().getMethod("clearCache");
            clearCache.invoke(automation);
        } catch (ReflectiveOperationException ignored) {
        }
    }
}
