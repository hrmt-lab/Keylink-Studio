import fs from "node:fs";
import http from "node:http";
import { acceptWebSocket } from "./ws-lite.mjs";

const args = new Map();
for (let index = 2; index < process.argv.length; index += 2) args.set(process.argv[index], process.argv[index + 1]);
const listenUrl = new URL(args.get("--listen"));
const token = fs.readFileSync(args.get("--ws-token-file"), "utf8").trim();
const server = http.createServer((request, response) => {
  if (request.url === "/readyz") {
    response.writeHead(200, { "content-type": "text/plain" });
    response.end("ready\n");
  } else {
    response.writeHead(404);
    response.end();
  }
});
server.on("upgrade", (request, socket, head) => {
  if (request.headers.authorization !== `Bearer ${token}`) {
    socket.end("HTTP/1.1 401 Unauthorized\r\nConnection: close\r\n\r\n");
    return;
  }
  const peer = acceptWebSocket(request, socket, head);
  peer.on("message", (message) => {
    if (message.type !== "text") return;
    const value = JSON.parse(message.data);
    if (value.method === "initialize") {
      peer.sendText(JSON.stringify({ jsonrpc: "2.0", id: value.id, result: { serverInfo: { name: "fake-app-server", version: "1" } } }));
    } else if (value.method === "thread/start") {
      peer.sendText(JSON.stringify({ jsonrpc: "2.0", id: value.id, result: { thread: { id: "fake-thread" } } }));
    } else if (value.method === "turn/start") {
      peer.sendText(JSON.stringify({ jsonrpc: "2.0", id: value.id, result: { turn: { id: "fake-turn" } } }));
      peer.sendText(JSON.stringify({ jsonrpc: "2.0", method: "turn/started", params: { threadId: "fake-thread", turnId: "fake-turn" } }));
      peer.sendText(JSON.stringify({ jsonrpc: "2.0", id: "fake-approval", method: "item/commandExecution/requestApproval", params: { threadId: "fake-thread", turnId: "fake-turn" } }));
    } else if (value.id === "fake-approval" && Object.hasOwn(value, "result")) {
      peer.sendText(JSON.stringify({ jsonrpc: "2.0", method: "turn/completed", params: { threadId: "fake-thread", turnId: "fake-turn", status: "completed" } }));
    }
  });
});
server.listen(Number(listenUrl.port), "127.0.0.1");

const originalParentPid = process.ppid;
const parentWatch = setInterval(() => {
  try {
    process.kill(originalParentPid, 0);
  } catch {
    clearInterval(parentWatch);
    server.close(() => process.exit(0));
  }
}, 100);

const stop = () => {
  clearInterval(parentWatch);
  server.close(() => process.exit(0));
};
process.once("SIGINT", stop);
process.once("SIGTERM", stop);
