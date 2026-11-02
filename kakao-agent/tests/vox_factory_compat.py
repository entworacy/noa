"""Exercise VoxController's real reflective facade loader with both APK contracts."""

import os
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class FactoryCompatibility(unittest.TestCase):
    def check_factory(self, package, name):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            sources = {
                "android/content/Context.java": "package android.content; public class Context {}",
                "com/kakao/talk/module/vox/contract/VoxModuleFacade.java": "package com.kakao.talk.module.vox.contract; public interface VoxModuleFacade {}",
                package.replace(".", "/")
                + "/"
                + name
                + ".java": f"package {package}; public class {name} {{ public {name}() {{}} "
                "public com.kakao.talk.module.vox.contract.VoxModuleFacade c(android.content.Context c) {"
                "return new com.kakao.talk.module.vox.contract.VoxModuleFacade() {}; }}",
                "Probe.java": """public class Probe {
                    public static void main(String[] args) throws Exception {
                        Class<?> controller = Class.forName("dev.noa.kakao.VoxController");
                        java.lang.reflect.Method m = controller.getDeclaredMethod("facade", Object.class);
                        m.setAccessible(true);
                        Object context = new android.content.Context();
                        Object result = m.invoke(null, context);
                        if (!(result instanceof com.kakao.talk.module.vox.contract.VoxModuleFacade))
                            throw new AssertionError("wrong facade");
                        if (result != m.invoke(null, context)) throw new AssertionError("not cached");
                    }
                }""",
            }
            for name_, content in sources.items():
                path = root / name_
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content)
            classes = pathlib.Path(
                os.environ.get(
                    "NOA_KAKAO_TEST_CLASSES", ROOT / "kakao-agent/build/classes"
                )
            )
            subprocess.run(
                [
                    "javac",
                    "-cp",
                    str(classes),
                    "-d",
                    tmp,
                    *map(str, root.rglob("*.java")),
                ],
                check=True,
            )
            subprocess.run(["java", "-cp", f"{tmp}:{classes}", "Probe"], check=True)

    def test_26_6_3_factory(self):
        self.check_factory("com.kakao.talk.vox", "VoxModuleFacadeFactory")

    def test_26_9_1_factory(self):
        self.check_factory("com.kakao.talk.vox.facade", "VoxFacadeModuleFactory")


if __name__ == "__main__":
    unittest.main()
