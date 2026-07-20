import crypto from "node:crypto";
import { EventEmitter } from "node:events";
import net from "node:net";

const WEBSOCKET_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const MAX_MESSAGE_BYTES = 16 * 1024 * 1024;

function websocketAccept(key) {
  return crypto.createHash("sha1").update(key + WEBSOCKET_GUID).digest("base64");
}

function encodeFrame(opcode, payload, masked) {
  const body = Buffer.isBuffer(payload) ? payload : Buffer.from(payload);
  const headerLength = body.length < 126 ? 2 : body.length <= 0xffff ? 4 : 10;
  const maskLength = masked ? 4 : 0;
  const frame = Buffer.allocUnsafe(headerLength + maskLength + body.length);
  frame[0] = 0x80 | opcode;
  let offset = 2;
  if (body.length < 126) {
    frame[1] = (masked ? 0x80 : 0) | body.length;
  } else if (body.length <= 0xffff) {
    frame[1] = (masked ? 0x80 : 0) | 126;
    frame.writeUInt16BE(body.length, 2);
    offset = 4;
  } else {
    frame[1] = (masked ? 0x80 : 0) | 127;
    frame.writeBigUInt64BE(BigInt(body.length), 2);
    offset = 10;
  }
  if (masked) {
    const mask = crypto.randomBytes(4);
    mask.copy(frame, offset);
    offset += 4;
    for (let index = 0; index < body.length; index += 1) {
      frame[offset + index] = body[index] ^ mask[index % 4];
    }
  } else {
    body.copy(frame, offset);
  }
  return frame;
}

export class WebSocketPeer extends EventEmitter {
  constructor(socket, { maskOutgoing, expectMasked, initialData = Buffer.alloc(0) }) {
    super();
    this.socket = socket;
    this.maskOutgoing = maskOutgoing;
    this.expectMasked = expectMasked;
    this.buffer = Buffer.alloc(0);
    this.fragmentOpcode = null;
    this.fragments = [];
    this.fragmentBytes = 0;
    this.closeSent = false;
    this.closed = false;
    socket.on("data", (chunk) => this.#consume(chunk));
    socket.on("error", (error) => this.emit("socketError", error));
    socket.on("close", () => this.#emitClose());
    if (initialData.length > 0) {
      queueMicrotask(() => this.#consume(initialData));
    }
  }

  sendText(text) {
    this.#send(0x1, Buffer.from(text, "utf8"));
  }

  sendBinary(data) {
    this.#send(0x2, Buffer.from(data));
  }

  close(code = 1000, reason = "") {
    if (this.closed || this.closeSent) return;
    const reasonBytes = Buffer.from(reason, "utf8").subarray(0, 123);
    const payload = Buffer.allocUnsafe(2 + reasonBytes.length);
    payload.writeUInt16BE(code, 0);
    reasonBytes.copy(payload, 2);
    this.closeSent = true;
    this.socket.write(encodeFrame(0x8, payload, this.maskOutgoing));
    this.socket.end();
  }

  terminate() {
    this.socket.destroy();
  }

  #send(opcode, payload) {
    if (this.closed || this.closeSent) throw new Error("WebSocket is closed");
    this.socket.write(encodeFrame(opcode, payload, this.maskOutgoing));
  }

  #protocolError(message) {
    this.emit("protocolError", new Error(message));
    this.close(1002, "protocol error");
  }

  #emitClose() {
    if (this.closed) return;
    this.closed = true;
    this.emit("close");
  }

  #consume(chunk) {
    if (this.closed) return;
    this.buffer = Buffer.concat([this.buffer, chunk]);
    while (this.buffer.length >= 2) {
      const first = this.buffer[0];
      const second = this.buffer[1];
      const fin = (first & 0x80) !== 0;
      const opcode = first & 0x0f;
      const masked = (second & 0x80) !== 0;
      if ((first & 0x70) !== 0 || masked !== this.expectMasked) {
        this.#protocolError("Invalid WebSocket frame flags");
        return;
      }
      let payloadLength = second & 0x7f;
      let offset = 2;
      if (payloadLength === 126) {
        if (this.buffer.length < 4) return;
        payloadLength = this.buffer.readUInt16BE(2);
        offset = 4;
      } else if (payloadLength === 127) {
        if (this.buffer.length < 10) return;
        const length64 = this.buffer.readBigUInt64BE(2);
        if (length64 > BigInt(MAX_MESSAGE_BYTES)) {
          this.close(1009, "message too large");
          return;
        }
        payloadLength = Number(length64);
        offset = 10;
      }
      const isControl = opcode >= 0x8;
      if ((isControl && (!fin || payloadLength > 125)) || payloadLength > MAX_MESSAGE_BYTES) {
        this.#protocolError("Invalid WebSocket payload length");
        return;
      }
      const frame = this.buffer;
      const maskOffset = offset;
      if (masked) offset += 4;
      if (frame.length < offset + payloadLength) return;
      const payload = Buffer.from(frame.subarray(offset, offset + payloadLength));
      this.buffer = frame.subarray(offset + payloadLength);
      if (masked) {
        const mask = frame.subarray(maskOffset, maskOffset + 4);
        for (let index = 0; index < payload.length; index += 1) {
          payload[index] ^= mask[index % 4];
        }
      }
      this.#handleFrame(fin, opcode, payload);
    }
  }

  #handleFrame(fin, opcode, payload) {
    if (opcode === 0x8) {
      if (!this.closeSent) {
        this.closeSent = true;
        this.socket.write(encodeFrame(0x8, payload, this.maskOutgoing));
      }
      this.socket.end();
      return;
    }
    if (opcode === 0x9) {
      this.socket.write(encodeFrame(0xA, payload, this.maskOutgoing));
      return;
    }
    if (opcode === 0xA) return;
    if (opcode !== 0x0 && opcode !== 0x1 && opcode !== 0x2) {
      this.#protocolError("Unsupported WebSocket opcode");
      return;
    }
    if (opcode === 0x0) {
      if (this.fragmentOpcode === null) {
        this.#protocolError("Unexpected continuation frame");
        return;
      }
    } else {
      if (this.fragmentOpcode !== null) {
        this.#protocolError("Interleaved fragmented message");
        return;
      }
      this.fragmentOpcode = opcode;
    }
    this.fragments.push(payload);
    this.fragmentBytes += payload.length;
    if (this.fragmentBytes > MAX_MESSAGE_BYTES) {
      this.close(1009, "message too large");
      return;
    }
    if (!fin) return;
    const messageOpcode = this.fragmentOpcode;
    const message = Buffer.concat(this.fragments, this.fragmentBytes);
    this.fragmentOpcode = null;
    this.fragments = [];
    this.fragmentBytes = 0;
    if (messageOpcode === 0x1) {
      this.emit("message", { type: "text", data: message.toString("utf8") });
    } else {
      this.emit("message", { type: "binary", data: message });
    }
  }
}

export function acceptWebSocket(request, socket, head = Buffer.alloc(0)) {
  const key = request.headers["sec-websocket-key"];
  const upgrade = String(request.headers.upgrade ?? "").toLowerCase();
  if (upgrade !== "websocket" || typeof key !== "string") {
    throw new Error("Invalid WebSocket upgrade request");
  }
  socket.write([
    "HTTP/1.1 101 Switching Protocols",
    "Upgrade: websocket",
    "Connection: Upgrade",
    `Sec-WebSocket-Accept: ${websocketAccept(key)}`,
    "",
    "",
  ].join("\r\n"));
  return new WebSocketPeer(socket, {
    maskOutgoing: false,
    expectMasked: true,
    initialData: head,
  });
}

export function connectWebSocket(rawUrl, { headers = {}, timeoutMs = 10_000 } = {}) {
  const url = new URL(rawUrl);
  if (url.protocol !== "ws:") throw new Error("Only ws:// URLs are supported");
  const port = Number(url.port || 80);
  const key = crypto.randomBytes(16).toString("base64");
  const expectedAccept = websocketAccept(key);
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host: url.hostname, port });
    let settled = false;
    let response = Buffer.alloc(0);
    const timer = setTimeout(() => fail(new Error("WebSocket handshake timed out")), timeoutMs);
    const fail = (error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      socket.destroy();
      reject(error);
    };
    socket.once("error", fail);
    socket.once("connect", () => {
      const path = `${url.pathname || "/"}${url.search}`;
      const requestHeaders = {
        Host: url.port ? `${url.hostname}:${url.port}` : url.hostname,
        Upgrade: "websocket",
        Connection: "Upgrade",
        "Sec-WebSocket-Key": key,
        "Sec-WebSocket-Version": "13",
        ...headers,
      };
      const lines = [`GET ${path} HTTP/1.1`];
      for (const [name, value] of Object.entries(requestHeaders)) lines.push(`${name}: ${value}`);
      socket.write(`${lines.join("\r\n")}\r\n\r\n`);
    });
    const onData = (chunk) => {
      response = Buffer.concat([response, chunk]);
      if (response.length > 64 * 1024) {
        fail(new Error("WebSocket handshake response is too large"));
        return;
      }
      const boundary = response.indexOf("\r\n\r\n");
      if (boundary < 0) return;
      const headerText = response.subarray(0, boundary).toString("latin1");
      const remaining = response.subarray(boundary + 4);
      const lines = headerText.split("\r\n");
      const status = Number(lines[0].split(" ")[1]);
      const responseHeaders = new Map();
      for (const line of lines.slice(1)) {
        const colon = line.indexOf(":");
        if (colon > 0) responseHeaders.set(line.slice(0, colon).trim().toLowerCase(), line.slice(colon + 1).trim());
      }
      if (status !== 101) {
        fail(new Error(`WebSocket handshake failed with HTTP ${status || "unknown"}`));
        return;
      }
      if (responseHeaders.get("sec-websocket-accept") !== expectedAccept) {
        fail(new Error("Invalid Sec-WebSocket-Accept response"));
        return;
      }
      settled = true;
      clearTimeout(timer);
      socket.off("data", onData);
      socket.off("error", fail);
      resolve(new WebSocketPeer(socket, {
        maskOutgoing: true,
        expectMasked: false,
        initialData: remaining,
      }));
    };
    socket.on("data", onData);
  });
}
