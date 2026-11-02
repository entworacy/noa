"""Run the production RichText preflight against synthetic Android/model contracts."""

import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class RichTextCompatibility(unittest.TestCase):
    def check_contract(self, getter, expected):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            sources = {
                "android/content/Context.java": """package android.content;
                    public class Context {
                        public android.content.pm.PackageManager getPackageManager() {
                            return new android.content.pm.PackageManager();
                        }
                    }""",
                "android/content/pm/PackageManager.java": """package android.content.pm;
                    public class PackageManager {
                        public static String version;
                        public PackageInfo getPackageInfo(String name, int flags) {
                            if (!name.equals("com.kakao.talk")) throw new AssertionError(name);
                            return new PackageInfo();
                        }
                    }""",
                "android/content/pm/PackageInfo.java": """package android.content.pm;
                    public class PackageInfo { public String versionName=PackageManager.version; }""",
                "com/kakao/talk/chat/richtext/RichText.java": """package com.kakao.talk.chat.richtext;
                    public class RichText {}""",
                "com/kakao/talk/manager/send/sending/ChatSendingLog.java": "package com.kakao.talk.manager.send.sending; public class ChatSendingLog {"
                + getter
                + "}",
                "dev/noa/kakao/Probe.java": """package dev.noa.kakao;
                    public class Probe {
                        public static void main(String[] args) throws Exception {
                            for (String version : new String[]{null,"","unknown","26","26.x.1",
                                    "25.99.9","26.6.3","26.8.9","026.08.0","-26.9","26.+9"}) {
                                android.content.pm.PackageManager.version=version;
                                try {
                                    RichTextSupport.require(new android.content.Context(), Probe.class.getClassLoader());
                                    throw new AssertionError("unsupported version: "+version);
                                } catch (IllegalStateException expected) {
                                    if (!expected.getMessage().contains("26.9")) throw expected;
                                }
                            }
                            for (String version : new String[]{"26.9.0","26.9.1","26.9.9","26.10.0","27.0.0"}) {
                                android.content.pm.PackageManager.version=version;
                                try {
                                    RichTextSupport.require(new android.content.Context(), Probe.class.getClassLoader());
                                    if (!Boolean.parseBoolean(args[0])) throw new AssertionError("missing contract");
                                } catch (NoSuchMethodException error) {
                                    if (Boolean.parseBoolean(args[0])) throw error;
                                }
                            }
                            try {
                                RichTextSupport.require(new android.content.Context(), new ClassLoader(null) {});
                                throw new AssertionError("missing model accepted");
                            } catch (ClassNotFoundException expected) {}
                        }
                    }""",
            }
            for name, contents in sources.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(contents)
            subprocess.run(
                [
                    "javac",
                    "--release",
                    "8",
                    "-d",
                    tmp,
                    str(ROOT / "kakao-agent/java/dev/noa/kakao/RichTextSupport.java"),
                    *map(str, root.rglob("*.java")),
                ],
                check=True,
            )
            subprocess.run(
                ["java", "-cp", tmp, "dev.noa.kakao.Probe", str(expected).lower()],
                check=True,
            )

    def test_supported_sending_log(self):
        self.check_contract(
            "public com.kakao.talk.chat.richtext.RichText d0() {return null;}", True
        )

    def test_missing_sending_log_contract(self):
        self.check_contract("public Object d0() {return null;}", False)

    def test_static_getter_is_not_a_sending_log_contract(self):
        self.check_contract(
            "public static com.kakao.talk.chat.richtext.RichText d0() {return null;}",
            False,
        )


if __name__ == "__main__":
    unittest.main()
