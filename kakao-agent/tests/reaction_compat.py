"""Native reaction adapter contracts using synthetic classes only."""
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]

class ReactionContract(unittest.TestCase):
    def test_native_arguments_callbacks_and_ambiguity(self):
        sources = {
            "kotlin/jvm/functions/Function0.java": "package kotlin.jvm.functions; public interface Function0 { Object invoke(); }",
            "kotlin/Unit.java": "package kotlin; public final class Unit { public static final Unit INSTANCE = new Unit(); }",
            "dev/noa/kakao/KakaoClassResolver.java": """package dev.noa.kakao;
                class KakaoClassResolver { static Object manager;
                static Object objectFor(String role) { if (!role.equals("reaction-manager")) throw new AssertionError(); return manager; } }""",
            "dev/noa/kakao/Bridge.java": """package dev.noa.kakao;
                class Bridge { static int calls; static long id;
                static void complete(long command, boolean ok, String detail) { if (!ok) throw new AssertionError(); calls++; id=command; } }""",
            "com/kakao/talk/module/emoticon/data/EmoticonReaction.java": """package com.kakao.talk.module.emoticon.data;
                public class EmoticonReaction { public final int type; public final String id;
                public EmoticonReaction(int type, String id) { this.type=type; this.id=id; } }""",
            "dev/noa/kakao/ReactionProbe.java": """package dev.noa.kakao;
                import java.util.List;
                import kotlin.jvm.functions.Function0;
                import com.kakao.talk.module.emoticon.data.EmoticonReaction;
                public class ReactionProbe {
                    static Function0 pending; static int calls;
                    public static class Params {
                        public Params(List add, List remove, List follow) {
                            if (add.size()!=1 || !remove.isEmpty() || !follow.isEmpty()) throw new AssertionError();
                            EmoticonReaction item=(EmoticonReaction)add.get(0);
                            if (item.type!=2 || !item.id.equals("item")) throw new AssertionError();
                        }
                    }
                    public static class Manager {
                        public void renamed(int kind, long room, long log, Long link, Function0 cb) {
                            if (kind!=1 || room!=10 || log!=9007199254740993L || link!=null) throw new AssertionError();
                            pending=cb; calls++;
                        }
                        public void other(Params params, long room, long log, Long link, long req, String id, Function0 cb) {
                            if (room!=10 || log!=9007199254740993L || link!=77L || req<=0 || id.isEmpty()) throw new AssertionError();
                            pending=cb; calls++;
                        }
                    }
                    public static class Ambiguous {
                        public void a(int t,long r,long l,Long k,Function0 cb) { throw new AssertionError(); }
                        public void b(int t,long r,long l,Long k,Function0 cb) { throw new AssertionError(); }
                    }
                    public static void main(String[] args) throws Exception {
                        KakaoClassResolver.manager=new Manager();
                        KakaoReactionResolver.react(10,9007199254740993L,0,42,1,0,null);
                        if (Bridge.calls!=0) throw new AssertionError("premature success");
                        if (pending.invoke()!=kotlin.Unit.INSTANCE || Bridge.id!=42) throw new AssertionError();
                        KakaoReactionResolver.react(10,9007199254740993L,77,43,0,2,"item");
                        if (Bridge.calls!=1) throw new AssertionError();
                        pending.invoke();
                        if (Bridge.calls!=2 || Bridge.id!=43 || calls!=2) throw new AssertionError();
                        for (Object manager : new Object[]{new Object(), new Ambiguous()}) {
                            KakaoClassResolver.manager=manager;
                            try { KakaoReactionResolver.react(10,20,0,44,1,0,null); throw new AssertionError(); }
                            catch (NoSuchMethodException expected) {}
                        }
                        for (long[] ids : new long[][]{{0,20,0},{10,0,0},{10,20,-1}}) {
                            try { KakaoReactionResolver.react(ids[0],ids[1],ids[2],45,1,0,null); throw new AssertionError(); }
                            catch (IllegalArgumentException expected) {}
                        }
                    }
                }""",
        }
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            for name, content in sources.items():
                p = root / name
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_text(content)
            production = ROOT / "kakao-agent/java/dev/noa/kakao"
            subprocess.run(["javac", "--release", "8", "-d", tmp,
                str(production / "KakaoReactionResolver.java"), str(production / "KakaoReflection.java"),
                *map(str, root.rglob("*.java"))], check=True)
            subprocess.run(["java", "-cp", tmp, "dev.noa/kakao/ReactionProbe".replace("/", ".")], check=True)

if __name__ == "__main__":
    unittest.main()
