package dev.noa;

import android.graphics.Rect;

import com.android.uiautomator.core.UiDevice;
import com.android.uiautomator.core.UiObject;
import com.android.uiautomator.core.UiObjectNotFoundException;
import com.android.uiautomator.core.UiSelector;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.regex.Pattern;

/** Locates the failed message bubble and clicks its version-specific retry control. */
final class UiResend {
    private static final String CHAT_LIST_ID =
        "com.kakao.talk:id/chat_log_recycler_list";
    private static final String RESEND_INDICATOR_ID = "com.kakao.talk:id/resend_indicator";
    private static final String DIRECT_RESEND_ID =
        "com.kakao.talk:id/circle_progress_layout";
    private static final String BUBBLE_LINE_ID =
        "com.kakao.talk:id/bubble_linearlayout";
    private static final String MESSAGE_BUBBLE_ID = "com.kakao.talk:id/bubble";
    private static final String ERROR_MESSAGE_ID = "com.kakao.talk:id/txt_message";
    private static final String[] ERROR_REPORT_PREFIXES = {
        "An unexpected error occurred.",
        "예기치 않은 오류가 발생했습니다."
    };
    private static final String[] CANCEL_LABELS = {"Cancel", "취소"};
    private static final int MAX_BACKWARD_SWIPES = 12;
    private static final long SWIPE_INTERVAL_NANOS = 250000000L;

    private UiResend() {}

    static String waitClickTarget(UiDevice device, String[] targets, long timeoutMs)
            throws Exception {
        long deadline = System.nanoTime() + timeoutMs * 1000000L;
        long nextSwipeAt = System.nanoTime() + SWIPE_INTERVAL_NANOS;
        int backwardSwipes = 0;
        String lastResult = "NOT_FOUND";
        while (true) {
            if (dismissKnownErrorReport(device)) {
                nextSwipeAt = System.nanoTime() + SWIPE_INTERVAL_NANOS;
                Thread.sleep(100);
                continue;
            }
            Candidate candidate = bestCandidate(targets);
            if (candidate != null) {
                if (!clickObject(device, candidate.target)) {
                    throw new IllegalStateException("resend target click rejected");
                }
                return candidate.direct ? "DIRECT" : "CONFIRM";
            }
            if (targets.length == 0) {
                int visibleTargets = visibleObjects(RESEND_INDICATOR_ID).size()
                    + visibleObjects(DIRECT_RESEND_ID).size();
                if (visibleTargets > 1) {
                    lastResult = "AMBIGUOUS";
                }
            }
            if (System.nanoTime() >= deadline) {
                return lastResult;
            }
            long now = System.nanoTime();
            if (targets.length > 0 && now >= nextSwipeAt) {
                Rect viewport = firstBounds(CHAT_LIST_ID);
                if (viewport != null) {
                    // KakaoTalk's reversed chat layout can initially anchor old failed
                    // cards below newer ones. A downward gesture exposes the latest rows.
                    boolean towardOlder = backwardSwipes >= MAX_BACKWARD_SWIPES;
                    swipe(device, viewport, towardOlder);
                    if (!towardOlder) {
                        backwardSwipes++;
                    }
                }
                nextSwipeAt = System.nanoTime() + SWIPE_INTERVAL_NANOS;
            }
            Thread.sleep(25);
        }
    }

    private static boolean dismissKnownErrorReport(UiDevice device) throws Exception {
        UiObject message = new UiObject(new UiSelector().resourceId(ERROR_MESSAGE_ID));
        if (!message.exists() || !startsWithAny(message.getText(), ERROR_REPORT_PREFIXES)) {
            return false;
        }
        for (String label : CANCEL_LABELS) {
            UiObject cancel = new UiObject(new UiSelector().text(label));
            if (cancel.exists()) {
                if (!clickObject(device, cancel)) {
                    throw new IllegalStateException("error report cancel click rejected");
                }
                return true;
            }
        }
        throw new IllegalStateException("known error report dialog has no cancel button");
    }

    private static boolean startsWithAny(String value, String[] prefixes) {
        if (value == null) {
            return false;
        }
        for (String prefix : prefixes) {
            if (value.startsWith(prefix)) {
                return true;
            }
        }
        return false;
    }

    private static Rect firstBounds(String resourceId) throws Exception {
        UiObject object = new UiObject(new UiSelector().resourceId(resourceId));
        return object.exists() ? object.getBounds() : null;
    }

    private static boolean swipe(UiDevice device, Rect bounds, boolean towardOlder) {
        int width = bounds.right - bounds.left;
        int height = bounds.bottom - bounds.top;
        if (width <= 0 || height <= 0) {
            return false;
        }
        int x = bounds.left + width / 2;
        int upper = bounds.top + height / 5;
        int lower = bounds.top + height * 3 / 4;
        return towardOlder
            ? device.swipe(x, lower, x, upper, 8)
            : device.swipe(x, upper, x, lower, 8);
    }

    private static Candidate bestCandidate(String[] targets) throws Exception {
        List<UiObject> indicators = visibleObjects(RESEND_INDICATOR_ID);
        List<UiObject> directTargets = visibleObjects(DIRECT_RESEND_ID);
        int targetCount = indicators.size() + directTargets.size();
        if (targetCount == 0 || (targets.length == 0 && targetCount != 1)) {
            return null;
        }
        Candidate best = bestCandidate(
            indicators,
            visibleBounds(BUBBLE_LINE_ID),
            false,
            targets,
            null
        );
        return bestCandidate(
            directTargets,
            visibleBounds(MESSAGE_BUBBLE_ID),
            true,
            targets,
            best
        );
    }

    private static Candidate bestCandidate(
        List<UiObject> targets,
        List<Rect> containers,
        boolean direct,
        String[] expectedTexts,
        Candidate best
    ) throws Exception {
        for (UiObject target : targets) {
            Rect targetBounds = target.getBounds();
            Rect bubble = smallestContainer(containers, targetBounds);
            if (bubble == null) {
                continue;
            }
            int score = expectedTexts.length == 0 ? 1 : scoreBubble(bubble, expectedTexts);
            if (score == 0) {
                continue;
            }
            if (best == null || score > best.score
                    || (score == best.score && targetBounds.bottom > best.bounds.bottom)
                    || (score == best.score && targetBounds.bottom == best.bounds.bottom
                        && direct && !best.direct)) {
                best = new Candidate(target, targetBounds, score, direct);
            }
        }
        return best;
    }

    private static int scoreBubble(Rect bubble, String[] targets) throws Exception {
        int score = 0;
        for (String target : targets) {
            if (target.isEmpty()) {
                continue;
            }
            String regex = "(?is).*" + Pattern.quote(target) + ".*";
            score += scoreMatches(
                new UiSelector().textMatches(regex), bubble, target, false
            );
            score += scoreMatches(
                new UiSelector().descriptionMatches(regex), bubble, target, true
            );
        }
        return score;
    }

    private static int scoreMatches(
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

    private static List<UiObject> visibleObjects(String resourceId) {
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

    private static List<Rect> visibleBounds(String resourceId) throws Exception {
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

    private static boolean clickObject(UiDevice device, UiObject target) throws Exception {
        final Rect bounds;
        try {
            bounds = target.getBounds();
        } catch (UiObjectNotFoundException ignored) {
            return false;
        }
        if (!device.click(
            (bounds.left + bounds.right) / 2,
            (bounds.top + bounds.bottom) / 2
        )) {
            return false;
        }
        device.waitForIdle(250);
        return true;
    }

    private static final class Candidate {
        private final UiObject target;
        private final Rect bounds;
        private final int score;
        private final boolean direct;

        private Candidate(UiObject target, Rect bounds, int score, boolean direct) {
            this.target = target;
            this.bounds = bounds;
            this.score = score;
            this.direct = direct;
        }
    }
}
