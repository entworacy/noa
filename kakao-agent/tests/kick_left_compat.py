"""Exercise the production departed-member adapter against synthetic JVM contracts."""

import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class LeftMemberKickContract(unittest.TestCase):
    def check_contract(self, getter):
        sources = {
            "com/kakao/talk/db/model/Friend.java": """package com.kakao.talk.db.model;
                public class Friend {
                    final long id;
                    public Friend(long id, String name, String image) { this.id=id; }
                    public long getId() { return id; }
                }""",
            "dev/noa/kakao/KakaoSignatureResolver.java": """package dev.noa.kakao;
                import java.util.*;
                public class KakaoSignatureResolver {
                    static ClassLoader appLoader() { return KakaoSignatureResolver.class.getClassLoader(); }
                    static List<Class<?>> sourceClasses(String source) {
                        throw new AssertionError("Unexpected member/repository lookup: " + source);
                    }
                }""",
            "dev/noa/kakao/KakaoClassResolver.java": """package dev.noa.kakao;
                public class KakaoClassResolver {
                    static Object manager;
                    static Object objectFor(String role) {
                        if (!role.equals("open-link-manager")) throw new AssertionError(role);
                        return manager;
                    }
                    static Object roomManager(Class<?> type) { throw new AssertionError(); }
                }""",
            "dev/noa/kakao/Bridge.java": """package dev.noa.kakao;
                public class Bridge {
                    static Boolean success;
                    static long id;
                    public static void complete(long command, boolean ok, String detail) {
                        id=command; success=ok;
                    }
                }""",
            "dev/noa/kakao/KickProbe.java": """package dev.noa.kakao;
                import java.lang.reflect.*;
                import com.kakao.talk.db.model.Friend;
                public class KickProbe {
                    public static class Room { public long getId() { return 10; } }
                    public static class Rooms { public Room get(long id) { return new Room(); } }
                    public interface Listener { void a(); void b(); }
                    public interface Access<T> { T target(); }
                    public static class Foreground {
                        static boolean reject;
                        static int calls;
                        public void renamedKick(Room room, Friend target, boolean ui, boolean report, Listener listener) {
                            calls++;
                            if (target.getId()!=9007199254740993L || ui || report) throw new AssertionError();
                            if (reject) listener.a(); else listener.b();
                        }
                    }
                    public static class Manager implements Access<Foreground> {
                        public Foreground target() { return new Foreground(); }
                    }
                    public static class Ambiguous {
                        public Foreground first() { return new Foreground(); }
                        public Foreground second() { return new Foreground(); }
                    }
                    public static void main(String[] args) throws Exception {
                        Class<?> binding=Class.forName("dev.noa.kakao.KakaoRoomResolver$RoomBinding");
                        Constructor<?> ctor=binding.getDeclaredConstructors()[0];
                        ctor.setAccessible(true);
                        Field field=KakaoRoomResolver.class.getDeclaredField("roomBinding");
                        field.setAccessible(true);
                        field.set(null, ctor.newInstance(new Rooms(), Rooms.class.getMethod("get", long.class)));
                        KakaoClassResolver.manager=new Manager();
                        KakaoLeftMemberKick.kick(10,9007199254740993L,42);
                        if (!Boolean.TRUE.equals(Bridge.success) || Bridge.id!=42) throw new AssertionError();
                        Foreground.reject=true;
                        KakaoLeftMemberKick.kick(10,9007199254740993L,43);
                        if (!Boolean.FALSE.equals(Bridge.success) || Bridge.id!=43) throw new AssertionError();
                        KakaoClassResolver.manager=new Ambiguous();
                        try { KakaoLeftMemberKick.kick(10,9007199254740993L,44); throw new AssertionError(); }
                        catch (NoSuchMethodException expected) {}
                        try { KakaoLeftMemberKick.kick(10,0,45); throw new AssertionError(); }
                        catch (IllegalArgumentException expected) {}
                        if (Foreground.calls!=2) throw new AssertionError("unexpected native invocation");
                    }
                }""",
        }
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            for name, content in sources.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content.replace("getId", getter))
            production = ROOT / "kakao-agent/java/dev/noa/kakao"
            subprocess.run(
                [
                    "javac",
                    "--release",
                    "8",
                    "-d",
                    tmp,
                    *[
                        str(production / (name + ".java"))
                        for name in [
                            "KakaoLeftMemberKick",
                            "KakaoRoomResolver",
                            "KakaoReflection",
                            "KakaoCallbacks",
                        ]
                    ],
                    *map(str, root.rglob("*.java")),
                ],
                check=True,
            )
            subprocess.run(["java", "-cp", tmp, "dev.noa.kakao.KickProbe"], check=True)

    def test_native_target_callbacks_and_ambiguous_accessors(self):
        self.check_contract("getId")

    def test_renamed_member_id_accessor(self):
        self.check_contract("renamedId")


if __name__ == "__main__":
    unittest.main()
