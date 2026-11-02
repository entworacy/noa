#!/usr/bin/env python3
"""Exercise the production request adapter against both published Iris model shapes."""

from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
ADAPTER = ROOT / "iris-agent/java/dev/noa/iris/ReplyRequestFactory.java"


def check(legacy):
    with tempfile.TemporaryDirectory(prefix="noa-iris-contract-") as temporary:
        root = Path(temporary)
        sources = {
            "kotlinx/serialization/json/JsonElement.java": """
package kotlinx.serialization.json;
public class JsonElement {}
""",
            "party/qwer/iris/model/ReplyRequest.java": """
package party.qwer.iris.model;
import kotlinx.serialization.json.JsonElement;
public final class ReplyRequest {
    public final TYPE type;
    public final String room, threadId;
    public final JsonElement data;
    public ReplyRequest(TYPE type, String room, JsonElement data, String threadId) {
        this.type = type; this.room = room; this.data = data; this.threadId = threadId;
    }
}
""".replace(
                "TYPE", "ReplyType" if legacy else "String"
            ),
            "ReplyRequestFactoryTest.java": """
import dev.noa.iris.ReplyRequestFactory;
import party.qwer.iris.model.ReplyRequest;
import kotlinx.serialization.json.JsonElement;
public class ReplyRequestFactoryTest {
    public static void main(String[] args) throws Exception {
        boolean legacy = Boolean.parseBoolean(args[0]);
        String expectedClass = legacy ? "party.qwer.iris.model.ReplyType" : "java.lang.String";
        require(ReplyRequestFactory.typeName().equals(expectedClass), "constructor selection");
        String[] types = {"text", "image", "image_multiple", "media", "video", "audio", "file", "future_type", "TEXT"};
        int passed = 0;
        for (int i = 0; i < types.length; i++) {
            JsonElement data = new JsonElement();
            boolean supported = !legacy || i < 3;
            try {
                ReplyRequest req = (ReplyRequest) ReplyRequestFactory.create(types[i], "9223372036854775807", data, "7");
                require(supported, "legacy unsupported type must fail");
                require(req.type.toString().equals(legacy ? types[i].toUpperCase(java.util.Locale.ROOT) : types[i]), "wire type");
                require(req.room.equals("9223372036854775807"), "room precision");
                require(req.data == data && req.threadId.equals("7"), "data identity and thread");
            } catch (IllegalArgumentException error) {
                require(!supported && error.getMessage().contains(types[i]), "unexpected type rejection");
            }
            passed++;
        }
        ReplyRequest sentinel = (ReplyRequest) ReplyRequestFactory.create("text", "42", new JsonElement(), null);
        require(sentinel.threadId == null, "optional thread");
        System.out.println("PASS " + (legacy ? "v0.32 enum" : "v0.33 string") + ": " + (passed + 2) + " request adapter checks");
    }
    private static void require(boolean ok, String message) {
        if (!ok) throw new AssertionError(message);
    }
}
""",
        }
        if legacy:
            sources["party/qwer/iris/model/ReplyType.java"] = """
package party.qwer.iris.model;
public enum ReplyType { TEXT, IMAGE, IMAGE_MULTIPLE }
"""
        paths = [str(ADAPTER)]
        for name, source in sources.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(source, encoding="utf-8")
            paths.append(str(path))
        subprocess.run(["javac", "--release", "8", "-d", str(root), *paths], check=True)
        subprocess.run(
            ["java", "-cp", str(root), "ReplyRequestFactoryTest", str(legacy).lower()],
            check=True,
        )


if __name__ == "__main__":
    check(True)
    check(False)
