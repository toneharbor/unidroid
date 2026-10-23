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

const child = spawn(process.env.UNIDROID ?? "target/release/unidroid", [
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

async function handle(_q: Question): Promise<Value | undefined> {
  // Answer app-specific Java calls here. Standard map calls are built in.
  return undefined;
}

try {
  const module = await request({
    cmd: "load", path: "tests/fixtures/armeabi-v7a/libsignutil.so",
  });
  const entries: Value[][] = [];
  for (const key of ["a", "b"]) {
    const keyHandle = await request({ cmd: "new_string", value: key });
    const value = await request({ cmd: "new_array", type: "B", base64: "Yg==" });
    entries.push([keyHandle, value]);
  }
  await request({ cmd: "onload", module });
  const aa = await request({ cmd: "new_string", value: "aa" });
  const bb = await request({ cmd: "new_string", value: "bb" });
  const map = await request({ cmd: "new_java_map", class: "java/util/HashMap", entries });
  const cc = await request({ cmd: "new_string", value: "cc" });
  const result = await request({
    cmd: "call_static",
    class: "com/anjuke/mobile/sign/SignUtil",
    method: "getSign0(Ljava/lang/String;Ljava/lang/String;Ljava/util/Map;Ljava/lang/String;I)Ljava/lang/String;",
    args: [aa, bb, map, cc, 10],
  });
  const sign = await request({ cmd: "string", handle: result });
  for (const handle of [aa, bb, map, cc, result, ...entries.flat()]) {
    await request({ cmd: "release", handle });
  }
  console.log(`sign: ${sign}`);
} finally {
  child.stdin.end();
  const code = await exited;
  if (code !== 0) process.exitCode = code ?? 1;
}
