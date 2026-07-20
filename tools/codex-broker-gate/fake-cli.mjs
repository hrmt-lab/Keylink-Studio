import { connectWebSocket } from "./ws-lite.mjs";

const args = new Map();
for (let index = 2; index < process.argv.length; index += 2) args.set(process.argv[index], process.argv[index + 1]);
const token = process.env[args.get("--token-env")];
const peer = await connectWebSocket(args.get("--remote"), { headers: { Authorization: `Bearer ${token}` } });
let approvalSeen = false;
const timeout = setTimeout(() => {
  process.stderr.write("Fake CLI timed out\n");
  peer.terminate();
  process.exit(1);
}, 5_000);

peer.on("message", (message) => {
  if (message.type !== "text") return;
  const value = JSON.parse(message.data);
  if (value.id === 1 && value.result) {
    peer.sendText(JSON.stringify({ jsonrpc: "2.0", method: "initialized", params: {} }));
    peer.sendText(JSON.stringify({ jsonrpc: "2.0", id: 2, method: "thread/start", params: {} }));
  } else if (value.id === 2 && value.result) {
    peer.sendText(JSON.stringify({ jsonrpc: "2.0", id: 3, method: "turn/start", params: { threadId: "fake-thread" } }));
  } else if (value.method === "item/commandExecution/requestApproval") {
    approvalSeen = true;
    peer.sendText(JSON.stringify({ jsonrpc: "2.0", id: value.id, result: { decision: "decline" } }));
  } else if (approvalSeen && value.method === "turn/completed") {
    clearTimeout(timeout);
    peer.close(1000, "fake test complete");
  }
});
peer.sendText(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { clientInfo: { name: "fake-cli", version: "1" } } }));
await new Promise((resolve) => peer.once("close", resolve));
