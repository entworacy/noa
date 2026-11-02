package dev.noa.iris;

import java.lang.reflect.Constructor;

/** Adapts the enum-based request model and the string-based model in Iris v0.33. */
public final class ReplyRequestFactory {
    private static final Constructor<?> CONSTRUCTOR = resolveConstructor();

    private ReplyRequestFactory() {}

    public static String typeName() {
        return CONSTRUCTOR.getParameterTypes()[0].getName();
    }

    public static Object create(String type, String room, Object data, String threadId)
            throws ReflectiveOperationException {
        Object wireType = type;
        Class<?> typeClass = CONSTRUCTOR.getParameterTypes()[0];
        if (typeClass != String.class) {
            String name;
            switch (type) {
                case "text": name = "TEXT"; break;
                case "image": name = "IMAGE"; break;
                case "image_multiple": name = "IMAGE_MULTIPLE"; break;
                default: throw new IllegalArgumentException("unsupported Iris /reply type: " + type);
            }
            wireType = typeClass.getField(name).get(null);
        }
        return CONSTRUCTOR.newInstance(wireType, room, data, threadId);
    }

    private static Constructor<?> resolveConstructor() {
        try {
            Class<?> request = Class.forName("party.qwer.iris.model.ReplyRequest");
            Class<?> data = Class.forName("kotlinx.serialization.json.JsonElement");
            try {
                return request.getConstructor(String.class, String.class, data, String.class);
            } catch (NoSuchMethodException legacy) {
                Class<?> type = Class.forName("party.qwer.iris.model.ReplyType");
                return request.getConstructor(type, String.class, data, String.class);
            }
        } catch (ReflectiveOperationException error) {
            throw new IllegalStateException("compatible Iris ReplyRequest constructor was not found", error);
        }
    }
}
