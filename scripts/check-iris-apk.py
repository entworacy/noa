#!/usr/bin/env python3
"""Check published Iris DEX contracts; this does not run Android hooks."""

import importlib.util
from pathlib import Path
import sys
import zipfile

sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location(
    "kakao_dex", Path(__file__).with_name("check-kakao-apk.py")
)
dex = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dex)


def audit(path):
    with zipfile.ZipFile(path) as archive:
        classes = {
            c["name"]: c
            for name in archive.namelist()
            if name.startswith("classes") and name.endswith(".dex")
            for c in dex.parse(archive.read(name))
        }
        abis = [
            n.split("/")[1]
            for n in archive.namelist()
            if n.endswith("/libsqlcipher.so")
        ]

    def descriptor(name):
        return "L" + name.replace(".", "/") + ";"

    def methods(owner, name):
        return [m for m in classes[descriptor(owner)]["methods"] if m[0] == name]

    def require_method(owner, name, arguments, returns, static=False):
        expected = "(" + " ".join(arguments) + ")" + returns
        found = [
            m
            for m in methods(owner, name)
            if m[1] == expected and bool(m[2] & 8) == static
        ]
        assert found, f"missing method: {owner}.{name}{expected}"
        print(f"PASS {owner}.{name}")

    def require_field(owner, name, field_type):
        assert any(
            n == name and t == field_type and flags & 8
            for n, t, flags in classes[descriptor(owner)]["fields"]
        ), f"missing static field: {owner}.{name}"
        print(f"PASS {owner}.{name}")

    string = descriptor("java.lang.String")
    obj = descriptor("java.lang.Object")
    element = descriptor("kotlinx.serialization.json.JsonElement")
    continuation = descriptor("kotlin.coroutines.Continuation")
    function1 = descriptor("kotlin.jvm.functions.Function1")
    call = descriptor("io.ktor.server.application.ApplicationCall")
    request = descriptor("io.ktor.server.request.ApplicationRequest")
    model = "party.qwer.iris.model.ReplyRequest"
    ctor = f"({string} {string} {element} {string})V"
    modern = any(m[1] == ctor for m in methods(model, "<init>"))
    wire_type = string if modern else descriptor("party.qwer.iris.model.ReplyType")
    print(
        f"{Path(path).name}: ReplyRequest.type={'String' if modern else 'enum'}; SQLCipher ABIs={abis}"
    )
    require_method(model, "<init>", [wire_type, string, element, string], "V")
    require_method(
        model + "$$serializer",
        "deserialize",
        [descriptor("kotlinx.serialization.encoding.Decoder")],
        descriptor(model),
    )
    require_method(
        "party.qwer.iris.Replier$Companion",
        "sendMessage",
        [string, "J", string, descriptor("java.lang.Long")],
        "V",
    )
    require_method("party.qwer.iris.Main", "main", ["[" + string], "V", True)
    require_method(
        "io.ktor.server.routing.RoutingRoot",
        "interceptor",
        [descriptor("io.ktor.util.pipeline.PipelineContext"), continuation],
        obj,
    )
    require_method(
        "kotlinx.serialization.json.JsonElementSerializer",
        "deserialize",
        [descriptor("kotlinx.serialization.encoding.Decoder")],
        element,
    )
    require_field(
        "kotlinx.serialization.json.JsonElementSerializer",
        "INSTANCE",
        descriptor("kotlinx.serialization.json.JsonElementSerializer"),
    )
    require_method(
        "kotlinx.serialization.json.JsonElementKt",
        "JsonPrimitive",
        [string],
        descriptor("kotlinx.serialization.json.JsonPrimitive"),
        True,
    )
    require_field("kotlin.Unit", "INSTANCE", descriptor("kotlin.Unit"))
    require_method(
        "kotlin.coroutines.intrinsics.IntrinsicsKt__IntrinsicsKt",
        "getCOROUTINE_SUSPENDED",
        [],
        obj,
        True,
    )
    require_method(
        "kotlin.ResultKt",
        "createFailure",
        [descriptor("java.lang.Throwable")],
        obj,
        True,
    )
    require_method("kotlin.ResultKt", "throwOnFailure", [obj], "V", True)
    require_method(
        "io.ktor.server.request.ApplicationReceiveFunctionsKt",
        "receiveChannel",
        [call, continuation],
        obj,
        True,
    )
    require_method(
        "io.ktor.utils.io.ByteReadChannelOperationsKt",
        "toByteArray",
        [descriptor("io.ktor.utils.io.ByteReadChannel"), continuation],
        obj,
        True,
    )
    require_method(
        "io.ktor.server.request.ApplicationRequestPropertiesKt",
        "getUri",
        [request],
        string,
        True,
    )
    require_method(
        "io.ktor.server.request.ApplicationRequestPropertiesKt",
        "getHttpMethod",
        [request],
        descriptor("io.ktor.http.HttpMethod"),
        True,
    )
    require_method(
        "io.ktor.server.response.ApplicationResponseFunctionsKt",
        "respondText",
        [
            call,
            string,
            descriptor("io.ktor.http.ContentType"),
            descriptor("io.ktor.http.HttpStatusCode"),
            function1,
            continuation,
        ],
        obj,
        True,
    )
    require_method(
        "io.ktor.server.routing.RoutingBuilderKt",
        "route",
        [descriptor("io.ktor.server.routing.Route"), string, function1],
        descriptor("io.ktor.server.routing.Route"),
        True,
    )
    for owner, method, arg, result in [
        (
            "io.ktor.http.ContentType$Companion",
            "parse",
            string,
            "io.ktor.http.ContentType",
        ),
        (
            "io.ktor.http.HttpStatusCode$Companion",
            "fromValue",
            "I",
            "io.ktor.http.HttpStatusCode",
        ),
    ]:
        require_field(owner.split("$")[0], "Companion", descriptor(owner))
        require_method(owner, method, [arg], descriptor(result))
    if not modern:
        for name in ["TEXT", "IMAGE", "IMAGE_MULTIPLE"]:
            require_field("party.qwer.iris.model.ReplyType", name, wire_type)


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit("Usage: check-iris-apk.py Iris.apk [Iris-older.apk ...]")
    for apk in sys.argv[1:]:
        audit(apk)
