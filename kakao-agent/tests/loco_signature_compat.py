"""Exercise the production LOCO resolver, including unnamed packet classes.

Fixtures are synthetic. NOA_TEST_JAVA may select an older JVM whose unnamed
classes have no Package object, matching Android's nullable Package metadata.
"""
import os
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class LocoSignatureContract(unittest.TestCase):
    def test_named_and_unnamed_packets_and_rejected_relationships(self):
        sources = {
            "android/app/ActivityThread.java": """package android.app;
                public class ActivityThread {
                    public static android.content.Context currentApplication() { return new android.content.Context(); }
                }""",
            "android/content/Context.java": """package android.content;
                public class Context {
                    public ClassLoader getClassLoader() { return getClass().getClassLoader(); }
                    public String getPackageName() { return "fixture"; }
                    public android.content.pm.PackageManager getPackageManager() { return new android.content.pm.PackageManager(); }
                }""",
            "android/content/pm/PackageManager.java": """package android.content.pm;
                public class PackageManager {
                    public PackageInfo getPackageInfo(String name, int flags) { return new PackageInfo(); }
                }""",
            "android/content/pm/PackageInfo.java": """package android.content.pm;
                public class PackageInfo { public long getLongVersionCode() { return 29260910L; } }""",
            "kotlin/coroutines/Continuation.java": "package kotlin.coroutines; public interface Continuation {}",
            "kotlin/jvm/functions/Function0.java": "package kotlin.jvm.functions; public interface Function0 {}",
            "kotlin/jvm/functions/Function2.java": "package kotlin.jvm.functions; public interface Function2 {}",
            "dev/noa/kakao/KakaoSignatureResolver.java": """package dev.noa.kakao;
                public class KakaoSignatureResolver {
                    public static String[] clients;
                    public static String[] sourceClassNames(String source) {
                        if (source.equals("LocoClient.kt")) return clients;
                        if (source.equals("LocoHeader.kt")) return new String[]{"fixture.Header"};
                        if (source.equals("LocoBody.kt")) return new String[]{"fixture.Body"};
                        throw new AssertionError(source);
                    }
                }""",
            "fixture/Header.java": """package fixture;
                public class Header {
                    public enum Kind { PING }
                    final int a,d; final short b; final Kind c;
                    public Header(int a,short b,Kind c,int d) { this.a=a;this.b=b;this.c=c;this.d=d; }
                }""",
            "fixture/Body.java": """package fixture;
                public class Body { final Object a; public Body(Object a) { this.a=a; } }""",
            "fixture/Base.java": """package fixture;
                public abstract class Base { final Header h=null; final Body b=null; }""",
        }
        for package in ("", "legacy"):
            folder = package + "/" if package else ""
            declaration = "package " + package + ";" if package else ""
            for packet in ("Request", "Response"):
                sources[folder + packet + ".java"] = declaration + " public final class " + packet + " extends fixture.Base {}"
            sources[folder + "Client.java"] = declaration + """
                import kotlin.coroutines.Continuation;
                import kotlin.jvm.functions.*;
                public final class Client {
                    public void send(Request r) {}
                    public static void bridge(Client c, Request r) {}
                    public Receiver factory(Request r,Object queue,boolean a,boolean b,Function2 f,Function0 g) { return null; }
                    public void a() {} public void b() {} public void c() {} public void d() {}
                    public void e() {} public void f() {} public void g() {}
                    public static class Receiver {
                        public Object receive(Response r,Continuation c) { return null; }
                        public Object await(Continuation c) { return null; }
                        public void close() {} public boolean alive() { return true; }
                    }
                    public static class MainResume {
                        Object a,b;
                        public Object invokeSuspend(Object value) { return value; }
                    }
                    public static class OtherResume {
                        public Object invokeSuspend(Object value) { return value; }
                    }
                }"""
        sources["foreign/Response.java"] = "package foreign; public final class Response extends fixture.Base {}"
        sources["InvalidClient.java"] = sources["Client.java"].replace("class Client", "class InvalidClient").replace("bridge(Client c", "bridge(InvalidClient c").replace("receive(Response r", "receive(foreign.Response r")
        sources["AmbiguousClient.java"] = sources["Client.java"].replace("class Client", "class AmbiguousClient").replace("bridge(Client c", "bridge(AmbiguousClient c").replace("public void send(Request r) {}", "public void send(Request r) {} public void duplicate(Request r) {}")
        sources["Probe.java"] = """
            import java.lang.reflect.Method;
            import dev.noa.kakao.*;
            public class Probe {
                static void index(String client) {
                    KakaoSignatureResolver.clients=new String[]{client,client+"$MainResume",client+"$OtherResume"};
                }
                static void resolves(String prefix) throws Exception {
                    index(prefix+"Client");
                    Object[] result=LocoSignatureResolver.resolve();
                    Method send=(Method)result[0],receive=(Method)result[1],resume=(Method)result[2];
                    if (!send.getName().equals("send") || !receive.getName().equals("receive")
                        || !resume.getDeclaringClass().getName().equals(prefix+"Client$MainResume")
                        || !result[3].toString().contains("versionCode=29260910")) throw new AssertionError();
                }
                public static void main(String[] args) throws Exception {
                    System.out.println("unnamed Package metadata: " + Request.class.getPackage());
                    resolves(""); resolves("legacy.");
                    for (String[] test : new String[][]{{"InvalidClient","packet relationship mismatch"},
                                                       {"AmbiguousClient","non-unique send or receive"}}) {
                        index(test[0]);
                        try { LocoSignatureResolver.resolve(); throw new AssertionError("invalid candidate accepted"); }
                        catch (NoSuchMethodException expected) {
                            if (!expected.getMessage().contains(test[1])) throw new AssertionError(expected);
                        }
                    }
                    System.out.println("LOCO signature contracts passed");
                }
            }"""
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            for name, content in sources.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content)
            production = ROOT / "kakao-agent/java/dev/noa/kakao"
            subprocess.run(["javac", "--release", "8", "-d", tmp,
                str(production / "LocoSignatureResolver.java"), str(production / "LocoPacketReader.java"),
                *map(str, root.rglob("*.java"))], check=True)
            subprocess.run([os.environ.get("NOA_TEST_JAVA", "java"), "-cp", tmp, "Probe"], check=True)


if __name__ == "__main__":
    unittest.main()
