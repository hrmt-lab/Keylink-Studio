import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";

const [portText, readyPath] = process.argv.slice(2);
const port = Number(portText);
if (!Number.isInteger(port) || port < 1024 || port > 65535 || !readyPath) {
  process.stderr.write("usage: node observer-self-test-server.mjs <port> <ready-path>\n");
  process.exit(2);
}

function textFrame(value) {
  const payload = Buffer.from(JSON.stringify(value), "utf8");
  if (payload.length < 126) {
    return Buffer.concat([Buffer.from([0x81, payload.length]), payload]);
  }
  if (payload.length <= 0xffff) {
    const header = Buffer.alloc(4);
    header[0] = 0x81;
    header[1] = 126;
    header.writeUInt16BE(payload.length, 2);
    return Buffer.concat([header, payload]);
  }
  throw new Error("self-test frame is unexpectedly large");
}

const server = http.createServer((_request, response) => {
  response.writeHead(426);
  response.end();
});

server.on("upgrade", (request, socket) => {
  const key = request.headers["sec-websocket-key"];
  if (typeof key !== "string") {
    socket.destroy();
    return;
  }
  const accept = crypto
    .createHash("sha1")
    .update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)
    .digest("base64");
  socket.write(
    "HTTP/1.1 101 Switching Protocols\r\n" +
      "Upgrade: websocket\r\n" +
      "Connection: Upgrade\r\n" +
      `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
  );

  setTimeout(() => {
    socket.write(textFrame({ id: 1, result: {} }));
  }, 100);
  setTimeout(() => {
    socket.write(
      textFrame({
        id: 99,
        method: "item/commandExecution/requestApproval",
        params: {
          itemId: "self-test-item",
          startedAtMs: 0,
          threadId: "self-test-thread",
          turnId: "self-test-turn",
        },
      }),
    );
  }, 250);
});

server.listen(port, "127.0.0.1", () => {
  fs.writeFileSync(readyPath, new Date().toISOString(), "ascii");
});

setTimeout(() => {
  process.stderr.write("self-test server timeout\n");
  process.exit(3);
}, 15000).unref();
