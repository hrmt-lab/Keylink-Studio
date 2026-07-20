import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { acceptWebSocket, connectWebSocket } from "./ws-lite.mjs";

function equalSecret(actual, expected) {
  const left = Buffer.from(actual ?? "", "utf8");
  const right = Buffer.from(expected, "utf8");
  return left.length === right.length && crypto.timingSafeEqual(left, right);
}

function bearerToken(request) {
  const value = request.headers.authorization;
  if (typeof value !== "string" || !value.startsWith("Bearer ")) return null;
  return value.slice("Bearer ".length);
}

function rejectUpgrade(socket, status, message) {
  const body = `${message}\n`;
  socket.end([
    `HTTP/1.1 ${status}`,
    "Connection: close",
    "Content-Type: text/plain; charset=utf-8",
    `Content-Length: ${Buffer.byteLength(body)}`,
    "",
    body,
  ].join("\r\n"));
}

function extractIdentifier(value, keys) {
  if (!value || typeof value !== "object") return undefined;
  for (const key of keys) {
    if (typeof value[key] === "string") return value[key];
  }
  for (const container of ["params", "result", "thread", "turn", "item"]) {
    const found = extractIdentifier(value[container], keys);
    if (found !== undefined) return found;
  }
  return undefined;
}

export function classifyJsonRpc(text) {
  let value;
  try {
    value = JSON.parse(text);
  } catch {
    return { kind: "non_json" };
  }
  if (Array.isArray(value)) return { kind: "batch", count: value.length };
  if (!value || typeof value !== "object") return { kind: "non_object_json" };
  const hasId = Object.prototype.hasOwnProperty.call(value, "id");
  const hasMethod = typeof value.method === "string";
  let kind = "unknown";
  if (hasMethod && hasId) kind = "request";
  else if (hasMethod) kind = "notification";
  else if (hasId && (Object.prototype.hasOwnProperty.call(value, "result") || Object.prototype.hasOwnProperty.call(value, "error"))) kind = "response";
  return {
    kind,
    ...(hasMethod ? { method: value.method } : {}),
    ...(hasId && (typeof value.id === "string" || typeof value.id === "number" || value.id === null) ? { id: value.id } : {}),
    ...(extractIdentifier(value, ["threadId", "thread_id"]) ? { threadId: extractIdentifier(value, ["threadId", "thread_id"]) } : {}),
    ...(extractIdentifier(value, ["turnId", "turn_id"]) ? { turnId: extractIdentifier(value, ["turnId", "turn_id"]) } : {}),
  };
}

function metadataWriter(metadataLog) {
  if (!metadataLog) return () => {};
  fs.mkdirSync(path.dirname(metadataLog), { recursive: true });
  return (record) => {
    fs.appendFileSync(metadataLog, `${JSON.stringify({ at: new Date().toISOString(), ...record })}\n`, { encoding: "utf8" });
  };
}

export async function createBroker({
  listenHost = "127.0.0.1",
  listenPort,
  upstreamUrl,
  clientToken,
  appServerToken,
  metadataLog,
}) {
  if (listenHost !== "127.0.0.1") throw new Error("Broker listen host must be 127.0.0.1");
  const upstream = new URL(upstreamUrl);
  if (upstream.protocol !== "ws:" || upstream.hostname !== "127.0.0.1") {
    throw new Error("App Server URL must use ws://127.0.0.1");
  }
  if (!clientToken || !appServerToken) throw new Error("Both capability tokens are required");
  if (equalSecret(clientToken, appServerToken)) throw new Error("CLI and App Server tokens must be different");
  const writeMetadata = metadataWriter(metadataLog);
  let active = null;
  let connecting = false;
  let stopping = false;
  const server = http.createServer((_request, response) => {
    response.writeHead(404, { "content-type": "text/plain; charset=utf-8" });
    response.end("WebSocket endpoint only\n");
  });

  server.on("upgrade", async (request, socket, head) => {
    socket.on("error", () => {});
    if (stopping) {
      rejectUpgrade(socket, "503 Service Unavailable", "Broker is stopping");
      return;
    }
    if (!equalSecret(bearerToken(request), clientToken)) {
      writeMetadata({ event: "downstream_auth_rejected" });
      rejectUpgrade(socket, "401 Unauthorized", "Unauthorized");
      return;
    }
    if (active !== null || connecting) {
      writeMetadata({ event: "additional_client_rejected" });
      rejectUpgrade(socket, "409 Conflict", "A CLI client is already connected");
      return;
    }
    connecting = true;
    let upstreamPeer;
    try {
      upstreamPeer = await connectWebSocket(upstreamUrl, {
        headers: { Authorization: `Bearer ${appServerToken}` },
      });
    } catch (error) {
      connecting = false;
      writeMetadata({ event: "upstream_connect_failed", error: error.message });
      rejectUpgrade(socket, "502 Bad Gateway", "App Server connection failed");
      return;
    }
    if (stopping || socket.destroyed) {
      connecting = false;
      upstreamPeer.close(1001, "downstream disconnected");
      if (!socket.destroyed) rejectUpgrade(socket, "503 Service Unavailable", "Broker is stopping");
      return;
    }
    let downstreamPeer;
    try {
      downstreamPeer = acceptWebSocket(request, socket, head);
    } catch (error) {
      connecting = false;
      upstreamPeer.close(1011, "downstream handshake failed");
      socket.destroy();
      return;
    }
    const connectionId = crypto.randomUUID();
    active = { connectionId, downstreamPeer, upstreamPeer };
    connecting = false;
    writeMetadata({ event: "connection_opened", connectionId });
    let finished = false;
    const finish = (origin) => {
      if (finished) return;
      finished = true;
      writeMetadata({ event: "connection_closed", connectionId, origin });
      downstreamPeer.close(1001, `${origin} closed`);
      upstreamPeer.close(1001, `${origin} closed`);
      if (active?.connectionId === connectionId) active = null;
    };
    const forward = (direction, destination, message) => {
      const classification = message.type === "text" ? classifyJsonRpc(message.data) : { kind: "binary" };
      writeMetadata({ event: "message", connectionId, direction, ...classification });
      try {
        if (message.type === "text") destination.sendText(message.data);
        else destination.sendBinary(message.data);
      } catch (error) {
        writeMetadata({ event: "forward_failed", connectionId, direction, error: error.message });
        finish(direction);
      }
    };
    downstreamPeer.on("message", (message) => forward("cli_to_app_server", upstreamPeer, message));
    upstreamPeer.on("message", (message) => forward("app_server_to_cli", downstreamPeer, message));
    downstreamPeer.on("close", () => finish("cli"));
    upstreamPeer.on("close", () => finish("app_server"));
    downstreamPeer.on("socketError", (error) => {
      writeMetadata({ event: "socket_error", connectionId, side: "cli", error: error.message });
      finish("cli");
    });
    upstreamPeer.on("socketError", (error) => {
      writeMetadata({ event: "socket_error", connectionId, side: "app_server", error: error.message });
      finish("app_server");
    });
  });

  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(listenPort, listenHost, resolve);
  });
  const address = server.address();
  writeMetadata({ event: "broker_ready", host: listenHost, port: address.port, upstream: upstreamUrl });
  return {
    host: listenHost,
    port: address.port,
    async close() {
      if (stopping) return;
      stopping = true;
      if (active) {
        active.downstreamPeer.close(1001, "broker stopping");
        active.upstreamPeer.close(1001, "broker stopping");
      }
      await new Promise((resolve) => server.close(resolve));
      writeMetadata({ event: "broker_stopped" });
    },
  };
}

function parseArguments(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined) throw new Error(`Invalid argument near ${name ?? "end"}`);
    values.set(name, value);
  }
  for (const required of ["--listen", "--upstream", "--client-token-file", "--app-server-token-file", "--metadata-log"]) {
    if (!values.has(required)) throw new Error(`Missing required argument: ${required}`);
  }
  const listen = new URL(values.get("--listen"));
  if (listen.protocol !== "ws:" || !listen.port) throw new Error("--listen must be a ws:// URL with an explicit port");
  return {
    listenHost: listen.hostname,
    listenPort: Number(listen.port),
    upstreamUrl: values.get("--upstream"),
    clientToken: fs.readFileSync(values.get("--client-token-file"), "utf8").trim(),
    appServerToken: fs.readFileSync(values.get("--app-server-token-file"), "utf8").trim(),
    metadataLog: values.get("--metadata-log"),
  };
}

async function main() {
  const broker = await createBroker(parseArguments(process.argv.slice(2)));
  process.stdout.write(`${JSON.stringify({ event: "broker_ready", host: broker.host, port: broker.port })}\n`);
  const stop = async () => {
    await broker.close();
    process.exit(0);
  };
  process.once("SIGINT", stop);
  process.once("SIGTERM", stop);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`${error.stack ?? error.message}\n`);
    process.exitCode = 1;
  });
}
