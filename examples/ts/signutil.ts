import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

type Value = null | boolean | number | string | Value[];
type Question = {
  op: string;
  class: string;
  name: string;
  signature: string;
  args: Value[];
};

const child = spawn(process.env.MINIDBG ?? "target/release/minidbg", [
  "serve", "--rootfs", "rootfs",
], { stdio: ["pipe", "pipe", "inherit"] });
const exited = new Promise<number | null>((resolve) => child.once("close", resolve));
const lines = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
const send = (message: object) => child.stdin.write(JSON.stringify(message) + "\n");

async function request(message: object): Promise<Value> {
  send(message);
  for (;;) {
    const reply = JSON.parse((await lines.next()).value!);
    if ("ok" in reply) return reply.ok;
    if ("error" in reply) throw new Error(reply.error);
    const value = await handle(reply.jni);
    send(value === undefined ? { cmd: "unhandled" } : { cmd: "answer", value });
  }
}

const entries = new Map<string, Value>();
let keys: Value;
let keySet: Value;

async function handle(q: Question): Promise<Value | undefined> {
  if (q.op !== "call") return undefined;
  switch (`${q.class}->${q.name}${q.signature}`) {
    case "java/util/HashMap->size()I": return entries.size;
    case "java/util/HashMap->keySet()Ljava/util/Set;": return keySet;
    case "java/util/Set->toArray()[Ljava/lang/Object;": return keys;
    case "java/util/HashMap->get(Ljava/lang/Object;)Ljava/lang/Object;": {
      const key = await request({ cmd: "string", handle: q.args[0] });
      return entries.get(String(key)) ?? 0;
    }
  }
}

try {
  const module = await request({
    cmd: "load", path: "tests/fixtures/armeabi-v7a/libsignutil.so",
  });
  const keyHandles: Value[] = [];
  for (const key of ["a", "b"]) {
    keyHandles.push(await request({ cmd: "new_string", value: key }));
    entries.set(key, await request({ cmd: "new_array", type: "B", base64: "Yg==" }));
  }
  keys = await request({ cmd: "new_object_array", class: "java/lang/Object", items: keyHandles });
  keySet = await request({ cmd: "new_object", class: "java/util/Set" });
  await request({ cmd: "onload", module });
  const aa = await request({ cmd: "new_string", value: "aa" });
  const bb = await request({ cmd: "new_string", value: "bb" });
  const map = await request({ cmd: "new_object", class: "java/util/HashMap" });
  const cc = await request({ cmd: "new_string", value: "cc" });
  const result = await request({
    cmd: "call_static",
    class: "com/anjuke/mobile/sign/SignUtil",
    method: "getSign0(Ljava/lang/String;Ljava/lang/String;Ljava/util/Map;Ljava/lang/String;I)Ljava/lang/String;",
    args: [aa, bb, map, cc, 10],
  });
  const sign = await request({ cmd: "string", handle: result });
  for (const handle of [aa, bb, map, cc, result, keys, keySet, ...keyHandles, ...entries.values()]) {
    await request({ cmd: "release", handle });
  }
  console.log(`sign: ${sign}`);
} finally {
  child.stdin.end();
  const code = await exited;
  if (code !== 0) process.exitCode = code ?? 1;
}
