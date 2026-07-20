import assert from "node:assert/strict";
import crypto from "node:crypto";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import fs from "node:fs";
import { createBroker } from "./broker.mjs";
import { acceptWebSocket, connectWebSocket } from "./ws-lite.mjs";

function onceMessage(peer, timeoutMs = 3_000) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Timed out waiting for WebSocket message")), timeoutMs);
    peer.once("message", (message) => {
      clearTimeout(timer);
      resolve(message);
    });
  });
}

function onceClose(peer, timeoutMs = 3_000) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Timed out waiting for WebSocket close")), timeoutMs);
    peer.once("close", () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

async function listen(server) {
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  return server.address().port;
}

const clientToken = crypto.randomBytes(32).toString("base64url");
const appServerToken = crypto.randomBytes(32).toString("base64url");
const temporaryDirectory = fs.mkdtempSync(path.join(os.tmpdir(), "keylink-broker-test-"));
const metadataLog = path.join(temporaryDirectory, "metadata.jsonl");
let upstreamPeer;
const upstreamConnected = new Promise((resolve) => {
  const upstreamServer = http.createServer();
  upstreamServer.on("upgrade", (request, socket, head) => {
    if (request.headers.authorization !== `Bearer ${appServerToken}`) {
      socket.end("HTTP/1.1 401 Unauthorized\r\nConnection: close\r\n\r\n");
      return;
    }
    upstreamPeer = acceptWebSocket(request, socket, head);
    resolve(upstreamPeer);
  });
  globalThis.upstreamServer = upstreamServer;
});

const upstreamPort = await listen(globalThis.upstreamServer);
await assert.rejects(
  createBroker({
    listenPort: 0,
    upstreamUrl: `ws://127.0.0.1:${upstreamPort}`,
    clientToken,
    appServerToken: clientToken,
  }),
  /must be different/,
);
const broker = await createBroker({
  listenPort: 0,
  upstreamUrl: `ws://127.0.0.1:${upstreamPort}`,
  clientToken,
  appServerToken,
  metadataLog,
});

try {
  await assert.rejects(
    connectWebSocket(`ws://127.0.0.1:${broker.port}`, { headers: { Authorization: "Bearer wrong-token" } }),
    /HTTP 401/,
  );

  const cliPeer = await connectWebSocket(`ws://127.0.0.1:${broker.port}`, {
    headers: { Authorization: `Bearer ${clientToken}` },
  });
  await upstreamConnected;

  const cliRequest = '{"jsonrpc":"2.0","id":7,"method":"thread/start","params":{"threadId":"thread-a"}}';
  const upstreamRequestPromise = onceMessage(upstreamPeer);
  cliPeer.sendText(cliRequest);
  assert.deepEqual(await upstreamRequestPromise, { type: "text", data: cliRequest });

  const appResponse = '{"jsonrpc":"2.0","id":7,"result":{"thread":{"id":"thread-a"}}}';
  const cliResponsePromise = onceMessage(cliPeer);
  upstreamPeer.sendText(appResponse);
  assert.deepEqual(await cliResponsePromise, { type: "text", data: appResponse });

  const notification = '{"jsonrpc":"2.0","method":"turn/started","params":{"threadId":"thread-a","turnId":"turn-a"}}';
  const cliNotificationPromise = onceMessage(cliPeer);
  upstreamPeer.sendText(notification);
  assert.deepEqual(await cliNotificationPromise, { type: "text", data: notification });

  const serverRequest = '{"jsonrpc":"2.0","id":"approval-1","method":"item/commandExecution/requestApproval","params":{"threadId":"thread-a","turnId":"turn-a"}}';
  const cliServerRequestPromise = onceMessage(cliPeer);
  upstreamPeer.sendText(serverRequest);
  assert.deepEqual(await cliServerRequestPromise, { type: "text", data: serverRequest });

  const serverResponse = '{"jsonrpc":"2.0","id":"approval-1","result":{"decision":"decline"}}';
  const upstreamResponsePromise = onceMessage(upstreamPeer);
  cliPeer.sendText(serverResponse);
  assert.deepEqual(await upstreamResponsePromise, { type: "text", data: serverResponse });

  await assert.rejects(
    connectWebSocket(`ws://127.0.0.1:${broker.port}`, { headers: { Authorization: `Bearer ${clientToken}` } }),
    /HTTP 409/,
  );

  const upstreamClosePromise = onceClose(upstreamPeer);
  cliPeer.close(1000, "test complete");
  await upstreamClosePromise;

  const secondCliPeer = await connectWebSocket(`ws://127.0.0.1:${broker.port}`, {
    headers: { Authorization: `Bearer ${clientToken}` },
  });
  const secondUpstreamPeer = upstreamPeer;
  const secondCliClosePromise = onceClose(secondCliPeer);
  secondUpstreamPeer.close(1011, "synthetic upstream failure");
  await secondCliClosePromise;

  const records = fs.readFileSync(metadataLog, "utf8").trim().split(/\r?\n/).map((line) => JSON.parse(line));
  const messages = records.filter((record) => record.event === "message");
  assert.equal(records.some((record) => record.event === "downstream_auth_rejected"), true);
  assert.equal(records.some((record) => record.event === "additional_client_rejected"), true);
  assert.equal(records.filter((record) => record.event === "connection_opened").length, 2);
  assert.equal(messages.some((record) => record.direction === "cli_to_app_server" && record.kind === "request" && record.method === "thread/start" && record.id === 7), true);
  assert.equal(messages.some((record) => record.direction === "app_server_to_cli" && record.kind === "notification" && record.method === "turn/started"), true);
  assert.equal(messages.some((record) => record.direction === "app_server_to_cli" && record.kind === "request" && record.method === "item/commandExecution/requestApproval" && record.id === "approval-1"), true);
  assert.equal(messages.some((record) => record.direction === "cli_to_app_server" && record.kind === "response" && record.id === "approval-1"), true);
  assert.equal(JSON.stringify(records).includes(clientToken), false);
  assert.equal(JSON.stringify(records).includes(appServerToken), false);

  process.stdout.write("Broker self-test passed.\n");
  process.stdout.write("Bidirectional request/response/notification forwarding: passed\n");
  process.stdout.write("Server request round-trip with unchanged IDs: passed\n");
  process.stdout.write("Separate downstream/upstream authentication: passed\n");
  process.stdout.write("Single-client guard and bidirectional disconnect propagation: passed\n");
  process.stdout.write("Sanitized metadata (no tokens or message bodies): passed\n");
} finally {
  await broker.close();
  await new Promise((resolve) => globalThis.upstreamServer.close(resolve));
  fs.rmSync(temporaryDirectory, { recursive: true, force: true });
}
