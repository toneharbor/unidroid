import json
import os
import subprocess
import sys

child = subprocess.Popen(
    [
        os.environ.get("MINIDBG", "target/release/minidbg"),
        "serve",
        "--rootfs",
        "rootfs",
    ],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    text=True,
)


def send(message):
    child.stdin.write(json.dumps(message) + "\n")
    child.stdin.flush()


def request(message):
    send(message)
    while True:
        reply = json.loads(child.stdout.readline())
        if "ok" in reply:
            return reply["ok"]
        if "error" in reply:
            raise RuntimeError(reply["error"])
        value = handle(reply["jni"])
        send({"cmd": "unhandled"} if value is None else {"cmd": "answer", "value": value})


entries = {}
keys = None
key_set = None


def handle(q):
    if q["op"] != "call":
        return None
    match f'{q["class"]}->{q["name"]}{q["signature"]}':
        case "java/util/HashMap->size()I":
            return len(entries)
        case "java/util/HashMap->keySet()Ljava/util/Set;":
            return key_set
        case "java/util/Set->toArray()[Ljava/lang/Object;":
            return keys
        case "java/util/HashMap->get(Ljava/lang/Object;)Ljava/lang/Object;":
            key = request({"cmd": "string", "handle": q["args"][0]})
            return entries.get(str(key), 0)
    return None


try:
    module = request({"cmd": "load", "path": "tests/fixtures/armeabi-v7a/libsignutil.so"})
    key_handles = []
    for key in ["a", "b"]:
        key_handles.append(request({"cmd": "new_string", "value": key}))
        entries[key] = request({"cmd": "new_array", "type": "B", "base64": "Yg=="})
    keys = request({"cmd": "new_object_array", "class": "java/lang/Object", "items": key_handles})
    key_set = request({"cmd": "new_object", "class": "java/util/Set"})
    request({"cmd": "onload", "module": module})
    aa = request({"cmd": "new_string", "value": "aa"})
    bb = request({"cmd": "new_string", "value": "bb"})
    map_handle = request({"cmd": "new_object", "class": "java/util/HashMap"})
    cc = request({"cmd": "new_string", "value": "cc"})
    result = request({
        "cmd": "call_static",
        "class": "com/anjuke/mobile/sign/SignUtil",
        "method": "getSign0(Ljava/lang/String;Ljava/lang/String;Ljava/util/Map;Ljava/lang/String;I)Ljava/lang/String;",
        "args": [aa, bb, map_handle, cc, 10],
    })
    sign = request({"cmd": "string", "handle": result})
    for obj in [aa, bb, map_handle, cc, result, keys, key_set, *key_handles, *entries.values()]:
        request({"cmd": "release", "handle": obj})
    print(f"sign: {sign}")
finally:
    child.stdin.close()
    code = child.wait()
    if code != 0:
        sys.exit(code)
