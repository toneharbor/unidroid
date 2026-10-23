import json
import os
import subprocess
import sys

child = subprocess.Popen(
    [
        os.environ.get("UNIDROID", "target/release/unidroid"),
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


def handle(q):
    # Answer app-specific Java calls here. Standard map calls are built in.
    return None


try:
    module = request({"cmd": "load", "path": "tests/fixtures/armeabi-v7a/libsignutil.so"})
    entries = []
    for key in ["a", "b"]:
        key_handle = request({"cmd": "new_string", "value": key})
        value = request({"cmd": "new_array", "type": "B", "base64": "Yg=="})
        entries.append([key_handle, value])
    request({"cmd": "onload", "module": module})
    aa = request({"cmd": "new_string", "value": "aa"})
    bb = request({"cmd": "new_string", "value": "bb"})
    map_handle = request({"cmd": "new_java_map", "class": "java/util/HashMap", "entries": entries})
    cc = request({"cmd": "new_string", "value": "cc"})
    result = request({
        "cmd": "call_static",
        "class": "com/anjuke/mobile/sign/SignUtil",
        "method": "getSign0(Ljava/lang/String;Ljava/lang/String;Ljava/util/Map;Ljava/lang/String;I)Ljava/lang/String;",
        "args": [aa, bb, map_handle, cc, 10],
    })
    sign = request({"cmd": "string", "handle": result})
    for obj in [aa, bb, map_handle, cc, result, *(obj for entry in entries for obj in entry)]:
        request({"cmd": "release", "handle": obj})
    print(f"sign: {sign}")
finally:
    child.stdin.close()
    code = child.wait()
    if code != 0:
        sys.exit(code)
