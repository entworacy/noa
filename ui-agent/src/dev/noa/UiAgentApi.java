package dev.noa;

import android.app.UiAutomation;
import android.content.ClipData;
import android.graphics.Rect;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityWindowInfo;

import com.android.uiautomator.core.Configurator;
import com.android.uiautomator.core.UiDevice;
import com.android.uiautomator.core.UiObject;
import com.android.uiautomator.core.UiScrollable;
import com.android.uiautomator.core.UiSelector;
import com.android.uiautomator.testrunner.UiAutomatorTestCase;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.List;

/** Fails startup when the legacy UIAutomator or hidden Android APIs drift. */
final class UiAgentApi {
    private UiAgentApi() {}

    static int verify() throws Exception {
        int sdkInt = Class.forName("android.os.Build$VERSION")
            .getField("SDK_INT")
            .getInt(null);
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
        requireMethod(bridgeClass, "getRootInActiveWindow", AccessibilityNodeInfo.class);
        requireMethod(UiAutomation.class, "getWindows", List.class);
        requireMethod(UiAutomation.class, "getRootInActiveWindow", AccessibilityNodeInfo.class);
        requireMethod(AccessibilityWindowInfo.class, "isActive", Boolean.TYPE);
        requireMethod(AccessibilityWindowInfo.class, "isFocused", Boolean.TYPE);
        requireMethod(AccessibilityWindowInfo.class, "getRoot", AccessibilityNodeInfo.class);
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
        requireMethod(AccessibilityNodeInfo.class, "getParent", AccessibilityNodeInfo.class);
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
        requireMethod(Class.forName("android.os.UserHandle"), "myUserId", Integer.TYPE);
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
        requireMethod(UiScrollable.class, "scrollForward", Boolean.TYPE, Integer.TYPE);

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
        requireMethod(UiDevice.class, "setCompressedLayoutHeirarchy", Void.TYPE, Boolean.TYPE);
        requireMethod(UiDevice.class, "dumpWindowHierarchy", Void.TYPE, String.class);
        return sdkInt;
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
