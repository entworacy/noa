"""Exercise the production unban adapter against synthetic native contracts."""

import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class UnbanContract(unittest.TestCase):
    def test_argument_order_callbacks_and_ambiguous_bindings(self):
        sources = {
            "dev/noa/kakao/KakaoClassResolver.java": """package dev.noa.kakao;
                public class KakaoClassResolver {
                    static Object manager;
                    static Object objectFor(String role) {
                        if (!role.equals("open-link-manager")) throw new AssertionError(role);
                        return manager;
                    }
                }""",
            "dev/noa/kakao/Bridge.java": """package dev.noa.kakao;
                public class Bridge {
                    static Boolean success;
                    static long id;
                    public static void complete(long command, boolean ok, String detail) {
                        id=command; success=ok;
                    }
                }""",
            "dev/noa/kakao/UnbanProbe.java": """package dev.noa.kakao;
                public class UnbanProbe {
                    public interface Access<T> { T target(); }
                    public static class Foreground {
                        static int calls;
                        static Runnable pending;
                        public void renamedUnban(long link, long room, long user, Runnable callback) {
                            if (link!=7 || room!=10 || user!=9007199254740993L) throw new AssertionError();
                            calls++; pending=callback;
                        }
                    }
                    public static class Manager implements Access<Foreground> {
                        public Foreground target() { return new Foreground(); }
                    }
                    public static class NullManager {
                        public Foreground target() { return null; }
                    }
                    public static class AmbiguousAccessors {
                        public Foreground first() { return new Foreground(); }
                        public Foreground second() { return new Foreground(); }
                    }
                    public static class DuplicateMethods extends Foreground {
                        public void another(long link, long room, long user, Runnable callback) {
                            throw new AssertionError();
                        }
                    }
                    public static class AmbiguousMethods {
                        public DuplicateMethods target() { return new DuplicateMethods(); }
                    }
                    public static void main(String[] args) throws Exception {
                        KakaoClassResolver.manager=new Manager();
                        KakaoUnbanResolver.unban(7,10,9007199254740993L,42);
                        if (Bridge.success!=null) throw new AssertionError("premature confirmation");
                        Foreground.pending.run();
                        if (!Boolean.TRUE.equals(Bridge.success) || Bridge.id!=42) throw new AssertionError();
                        for (Object manager : new Object[]{new Object(),new AmbiguousAccessors(),new AmbiguousMethods()}) {
                            KakaoClassResolver.manager=manager;
                            try { KakaoUnbanResolver.unban(7,10,9007199254740993L,43); throw new AssertionError(); }
                            catch (NoSuchMethodException expected) {}
                        }
                        KakaoClassResolver.manager=new NullManager();
                        try { KakaoUnbanResolver.unban(7,10,9007199254740993L,44); throw new AssertionError(); }
                        catch (IllegalStateException expected) {}
                        for (long[] ids : new long[][]{{0,10,20},{7,0,20},{7,10,-1}}) {
                            try { KakaoUnbanResolver.unban(ids[0],ids[1],ids[2],45); throw new AssertionError(); }
                            catch (IllegalArgumentException expected) {}
                        }
                        if (Foreground.calls!=1) throw new AssertionError("unexpected native invocation");
                    }
                }""",
        }
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            for name, content in sources.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content)
            production = ROOT / "kakao-agent/java/dev/noa/kakao"
            subprocess.run([
                "javac", "--release", "8", "-d", tmp,
                str(production / "KakaoUnbanResolver.java"),
                str(production / "KakaoReflection.java"),
                *map(str, root.rglob("*.java")),
            ], check=True)
            subprocess.run(["java", "-cp", tmp, "dev.noa.kakao.UnbanProbe"], check=True)


if __name__ == "__main__":
    unittest.main()
