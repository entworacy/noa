#!/usr/bin/env python3
"""Static DEX contract audit. Does not establish on-device hook compatibility.
Usage: python3 scripts/check-kakao-apk.py base.apk [split.apk ...]
Also run the ignored Rust apk_signature_contract test for operation indexing.
"""

import json
import struct
import sys
import zipfile


def parse(b):
    if (
        len(b) < 112
        or b[:4] != b"dex\n"
        or b[4:7] not in (b"035", b"037", b"038", b"039", b"040")
    ):
        raise ValueError("Unsupported or truncated standard DEX header")
    if struct.unpack_from("<I", b, 40)[0] != 0x12345678:
        raise ValueError("Only little-endian DEX is supported")
    u = lambda o: struct.unpack_from("<I", b, o)[0]
    h = lambda o: struct.unpack_from("<H", b, o)[0]

    def leb(o):
        n = s = 0
        while True:
            v = b[o]
            o += 1
            n |= (v & 127) << s
            s += 7
            if v < 128:
                return n, o

    def string(o):
        _, o = leb(o)
        return b[o : b.index(0, o)].decode("utf8", "replace")

    strings = [string(u(u(60) + 4 * i)) for i in range(u(56))]
    types = [strings[u(u(68) + 4 * i)] for i in range(u(64))]

    def typelist(o):
        return [types[h(o + 4 + 2 * i)] for i in range(u(o))] if o else []

    protos = []
    for i in range(u(72)):
        o = u(76) + 12 * i
        protos.append("(" + " ".join(typelist(u(o + 8))) + ")" + types[u(o + 4)])
    methods = []
    for i in range(u(88)):
        o = u(92) + 8 * i
        methods.append((strings[u(o + 4)], protos[h(o + 2)]))
    fields = []
    for i in range(u(80)):
        o = u(84) + 8 * i
        fields.append((strings[u(o + 4)], types[h(o + 2)]))
    rows = []
    for i in range(u(96)):
        o = u(100) + 32 * i
        idx, flags, sup, ifs, src, ann, data, vals = struct.unpack_from("<8I", b, o)
        row = {
            "name": types[idx],
            "source": strings[src] if src != 0xFFFFFFFF else None,
            "super": types[sup] if sup != 0xFFFFFFFF else None,
            "interfaces": typelist(ifs),
            "flags": flags,
            "fields": [],
            "methods": [],
        }
        if data:
            counts = []
            for _ in range(4):
                n, data = leb(data)
                counts.append(n)
            for group, count in enumerate(counts):
                index = 0
                for _ in range(count):
                    delta, data = leb(data)
                    index += delta
                    access, data = leb(data)
                    if group >= 2:
                        code, data = leb(data)
                        row["methods"].append([*methods[index], access])
                    else:
                        row["fields"].append([*fields[index], access])
        rows.append(row)
    return rows


def audit(paths):
    rows = []
    for path in paths:
        with zipfile.ZipFile(path) as z:
            for name in z.namelist():
                if name.startswith("classes") and name.endswith(".dex"):
                    rows.extend(parse(z.read(name)))
    by = {c["name"]: c for c in rows}

    def cls(name):
        return by["L" + name.replace(".", "/") + ";"]

    def has(c, name, desc):
        return any(m[0] == name and m[1] == desc for m in c["methods"])

    def require(ok, message):
        if not ok:
            raise AssertionError(message)
        print("PASS", message)

    def source_classes(source):
        return [c for c in rows if c["source"] == source]

    def largest_outer(source):
        return max(
            (
                c
                for c in source_classes(source)
                if "$" not in c["name"] and not c["flags"] & (512 | 16384)
            ),
            key=lambda c: len(c["methods"]) * 4 + len(c["fields"]),
        )

    def instance_methods(c):
        return [
            (n, d[1 : d.index(")")].split(), d.split(")")[1], f)
            for n, d, f in c["methods"]
            if not f & 8 and not n.startswith("<")
        ]

    manager = largest_outer("ChatRoomListManager.kt")
    legacy = []
    for n, d, f in manager["fields"]:
        if not f & 8:
            continue
        if d == manager["name"]:
            legacy.append(n)
        elif d.startswith(manager["name"][:-1] + "$") and d in by:
            legacy.extend(
                mn
                for mn, mp, mr, mf in instance_methods(by[d])
                if not mp and mr == manager["name"]
            )
    providers = [
        (c["name"], n)
        for c in source_classes("ChatRoomListManagerProvider.kt")
        for n, d, f in c["methods"]
        if f & 8 and d == "()" + manager["name"]
    ]
    require(
        bool(legacy) or len(providers) == 1,
        "room manager instance route: legacy="
        + str(legacy)
        + ", providers="
        + str(providers),
    )
    friend = cls("com.kakao.talk.db.model.Friend")
    friend_id_owner = friend
    def has_id_accessor(owner):
        return any(not flags & 8 and desc == "()J" for _, desc, flags in owner["methods"])
    while not has_id_accessor(friend_id_owner) and friend_id_owner["super"] in by:
        friend_id_owner = by[friend_id_owner["super"]]
    require(
        has(friend, "<init>", "(J Ljava/lang/String; Ljava/lang/String;)V")
        and has_id_accessor(friend_id_owner),
        "departed kick Friend ID contract",
    )
    olk = largest_outer("OlkManager.kt")
    kick_accessors = []
    for name, params, result, flags in instance_methods(olk):
        if (
            params
            or flags & (64 | 4096)
            or result not in by
            or by[result]["flags"] & 512
        ):
            continue
        matches = [
            m
            for m in instance_methods(by[result])
            if m[2] == "V"
            and len(m[1]) == 5
            and m[1][1:4] == [friend["name"], "Z", "Z"]
            and m[1][4] in by
            and by[m[1][4]]["flags"] & 512
        ]
        if len(matches) == 1:
            kick_accessors.append((name, result, matches[0][0]))
    require(
        len(kick_accessors) == 1, "unique native kick accessor: " + str(kick_accessors)
    )
    unban_accessors = []
    for name, params, result, flags in instance_methods(olk):
        if params or flags & (64 | 4096) or result not in by or by[result]["flags"] & 512:
            continue
        matches = [m for m in instance_methods(by[result]) if m[2] == "V"
                   and m[1] == ["J", "J", "J", "Ljava/lang/Runnable;"]]
        if len(matches) == 1:
            unban_accessors.append((name, result, matches[0][0]))
    require(len(unban_accessors) == 1 and unban_accessors[0][:2] == kick_accessors[0][:2],
            "unique native unban accessor: " + str(unban_accessors))
    context = cls("kotlin.coroutines.EmptyCoroutineContext")
    require(
        any(
            n == "C" and d == context["name"] and f & 8 for n, d, f in context["fields"]
        ),
        "EmptyCoroutineContext.C used by suspend operations",
    )
    require(
        has(cls("com.kakao.talk.manager.send.sending.ChatSendingLog"), "getId", "()J"),
        "sending-log getId",
    )
    link = cls("com.kakao.talk.openlink.db.model.OpenLink")
    for getter in ["getName", "getUrl"]:
        require(has(link, getter, "()Ljava/lang/String;"), "OpenLink." + getter)
    require(
        len(
            [
                m
                for m in instance_methods(manager)
                if m[1] == [link["name"]] + ["Ljava/lang/String;"] * 3
            ]
        )
        == 1,
        "create-open-chat argument contract",
    )
    api = largest_outer("ChatRoomApiHelper.kt")
    companion = max(
        (
            by[d]
            for n, d, f in api["fields"]
            if f & 8 and d.startswith(api["name"][:-1] + "$") and d in by
        ),
        key=lambda c: len(c["methods"]),
    )
    require(
        len(
            [
                m
                for m in instance_methods(companion)
                if m[1] == ["J", "Ljava/util/List;", "Lkotlin/coroutines/Continuation;"]
                and m[2] == "Ljava/lang/Object;"
            ]
        )
        == 1,
        "GETMEM unique suspend signature",
    )
    for db in ["MasterDatabase", "SecondaryDatabase"]:
        c = cls("com.kakao.talk.database." + db)
        trackers = []
        while c:
            for n, params, ret, f in instance_methods(c):
                if params or ret not in by:
                    continue
                if any(
                    p == ["[Ljava/lang/String;", "Z", "Ljava/util/concurrent/Callable;"]
                    for mn, p, r, mf in instance_methods(by[ret])
                ):
                    trackers.append((c["name"], n, ret))
            c = by.get(c["super"])
        require(bool(trackers), db + " invalidation tracker/LiveData factory")

    facade = "Lcom/kakao/talk/module/vox/contract/VoxModuleFacade;"
    factories = [
        "com.kakao.talk.vox.facade.VoxFacadeModuleFactory",
        "com.kakao.talk.vox.VoxModuleFacadeFactory",
    ]
    found = []
    for name in factories:
        if "L" + name.replace(".", "/") + ";" not in by:
            continue
        c = cls(name)
        if has(c, "<init>", "()V") and any(
            m[1] == "(Landroid/content/Context;)" + facade for m in c["methods"]
        ):
            found.append(name)
    require(bool(found), "VOX factory: " + str(found))
    contract = by[facade]
    require(
        has(
            contract,
            "ensurePermissionsAndMakeCall",
            "(Landroid/content/Context; Lcom/kakao/talk/module/vox/domain/model/cecall/CecallMakingInfo; Lcom/kakao/talk/module/vox/domain/model/cecall/CecallOutgoingTrigger;)V",
        ),
        "VOX make-call contract",
    )
    for getter in ["getCecallManager", "getVoiceroomManager"]:
        method = next(
            m for m in contract["methods"] if m[0] == getter and m[1].startswith("()")
        )
        manager = by[method[1][2:]]
        require(has(manager, "isInIdle", "()Z"), getter + " idle contract")
        if getter == "getVoiceroomManager":
            require(
                has(
                    manager,
                    "makeVoiceroom",
                    "(Landroid/content/Context; J Ljava/lang/String; Lkotlin/jvm/functions/Function0; Lkotlin/jvm/functions/Function1;)V",
                ),
                "VOX room creation contract",
            )
            require(
                has(manager, "leaveVoiceroom", "(Lkotlin/jvm/functions/Function1;)V"),
                "VOX room leave contract",
            )
        else:
            require(
                has(manager, "endCallByLeaveChatRoom", "(J)V"),
                "VOX call leave contract",
            )
    require(
        any(
            has(
                c,
                "joinVoiceroom",
                "(J J Ljava/lang/String; Ljava/lang/String; I Lkotlin/jvm/functions/Function0; Lkotlin/jvm/functions/Function1;)V",
            )
            for c in rows
        ),
        "VOX room join signature exists (receiver linkage needs runtime verification)",
    )
    making = cls("com.kakao.talk.module.vox.domain.model.cecall.CecallMakingInfo")
    require(
        any(
            m[0] == "<init>"
            and m[1].startswith("(J J J Ljava/util/List; L")
            and m[1].endswith("; Z Z Z Z)V")
            for m in making["methods"]
        ),
        "VOX nine-argument constructor",
    )
    for name in [
        "com.kakao.talk.database.MasterDatabase",
        "com.kakao.talk.database.SecondaryDatabase",
        "com.kakao.talk.vox.ui.voiceroom.VoiceroomActivity",
        "com.kakao.talk.module.vox.domain.model.cecall.CecallOutgoingTrigger$Auto",
    ]:
        require("L" + name.replace(".", "/") + ";" in by, "class " + name)
    matches = []
    for c in rows:
        if c["source"] != "LocoClient.kt" or not c["flags"] & 16:
            continue
        for name, desc, flags in c["methods"]:
            params, ret = desc.split(")")
            params = params[1:].split()
            if (
                flags & 8
                or len(params) != 6
                or params[2:]
                != [
                    "Z",
                    "Z",
                    "Lkotlin/jvm/functions/Function2;",
                    "Lkotlin/jvm/functions/Function0;",
                ]
                or not ret.startswith(c["name"][:-1] + "$")
            ):
                continue
            send = [
                m
                for m in c["methods"]
                if not m[2] & 8 and m[1] == "(" + params[0] + ")V"
            ]
            receive = [
                m
                for m in by[ret]["methods"]
                if not m[2] & 8
                and len(m[1].split(")")[0].split()) == 2
                and m[1].endswith("Lkotlin/coroutines/Continuation;)Ljava/lang/Object;")
            ]
            if len(send) != 1 or len(receive) != 1:
                continue
            response = receive[0][1][1:].split()[0]
            if by[params[0]]["super"] != by[response]["super"]:
                continue
            matches.append((c["name"], send[0][:2], ret, receive[0][:2]))
    require(
        len(matches) == 1, "LOCO structural send/receive candidate: " + str(matches)
    )
    for descriptor in [matches[0][1][1][1:-2], matches[0][3][1][1:].split()[0]]:
        fields = []
        c = by[descriptor]
        while c:
            fields.extend((n, d, f) for n, d, f in c["fields"] if not f & 8)
            c = by.get(c["super"])
        header_fields = [
            x for x in fields if by.get(x[1], {}).get("source") == "LocoHeader.kt"
        ]
        body_fields = [
            x for x in fields if by.get(x[1], {}).get("source") == "LocoBody.kt"
        ]
        require(
            len(header_fields) == 1 and len(body_fields) == 1,
            descriptor + " unique packet header/body fields",
        )
        header = by[header_fields[0][1]]
        body = by[body_fields[0][1]]
        hf = {n: d for n, d, f in header["fields"] if not f & 8}
        bf = {n: d for n, d, f in body["fields"] if not f & 8}
        require(
            hf.get("a") == "I"
            and hf.get("b") == "S"
            and hf.get("d") == "I"
            and by.get(hf.get("c"), {}).get("flags", 0) & 16384
            and has(header, "<init>", "(I S " + hf["c"] + " I)V"),
            descriptor + " header id/status/method/length layout",
        )
        require(
            bf.get("a", "").startswith("L")
            and has(body, "<init>", "(" + bf["a"] + ")V"),
            descriptor + " BSON body layout",
        )
    request = matches[0][1][1][1:-2]
    client = by[matches[0][0]]
    require(len([m for m in client["methods"] if not m[2] & 8 and m[1] == "()Z"]) == 1,
            "LOCO sender unique client liveness accessor")
    builders = []
    for c in rows:
        if c["source"] != "LocoReq.kt":
            continue
        if (has(c, "<init>", "(" + hf["c"] + " I S)V")
            and len([m for m in c["methods"] if not m[2] & 8 and m[1] == "()" + request]) == 1
            and len([m for m in c["methods"] if not m[2] & 8 and m[1] == "(Ljava/lang/String; Ljava/lang/Object;)" + c["name"]]) == 1):
            builders.append(c["name"])
    require(len(builders) == 1, "LOCO request builder: " + str(builders))
    generators = []
    for c in rows:
        if c["source"] != "LocoJob.kt":
            continue
        if (any(d == "Ljava/util/concurrent/atomic/AtomicInteger;" and f & 8 for n,d,f in c["fields"])
            and any(d == c["name"] and f & 8 for n,d,f in c["fields"])
            and len([m for m in c["methods"] if not m[2] & 8 and m[1] == "()I"]) == 1):
            generators.append(c["name"])
    require(len(generators) == 1, "LOCO shared ID allocator: " + str(generators))
    print(
        "Static checks only: ART/LSPlant, reflection initialization, callbacks and real operations require a device."
    )


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    audit(sys.argv[1:])
