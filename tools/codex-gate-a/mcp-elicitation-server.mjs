import readline from "node:readline";

const rl = readline.createInterface({
  input: process.stdin,
  crlfDelay: Infinity,
});

let nextServerRequestId = 1;
const pendingElicitations = new Map();

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

function sendResult(id, result) {
  send({ jsonrpc: "2.0", id, result });
}

function sendError(id, code, message) {
  send({ jsonrpc: "2.0", id, error: { code, message } });
}

function requestElicitation(toolCallId) {
  const elicitationId = `gate-a-elicitation-${nextServerRequestId++}`;
  pendingElicitations.set(elicitationId, toolCallId);
  send({
    jsonrpc: "2.0",
    id: elicitationId,
    method: "elicitation/create",
    params: {
      message: "Gate A MCP elicitation delivery test",
      requestedSchema: {
        type: "object",
        properties: {
          confirmation: {
            type: "boolean",
            title: "Confirm Gate A test",
            description: "Choose either value to resolve this test request.",
          },
        },
        required: ["confirmation"],
      },
    },
  });
}

rl.on("line", (line) => {
  if (!line.trim()) return;

  let message;
  try {
    message = JSON.parse(line);
  } catch {
    return;
  }

  if (!message.method && message.id != null) {
    const toolCallId = pendingElicitations.get(String(message.id));
    if (toolCallId == null) return;

    pendingElicitations.delete(String(message.id));
    if (message.error) {
      sendResult(toolCallId, {
        content: [{ type: "text", text: "Gate A elicitation returned an error." }],
        isError: true,
      });
    } else {
      sendResult(toolCallId, {
        content: [{ type: "text", text: "Gate A elicitation was resolved." }],
        isError: false,
      });
    }
    return;
  }

  switch (message.method) {
    case "initialize":
      sendResult(message.id, {
        protocolVersion: message.params?.protocolVersion ?? "2025-06-18",
        capabilities: { tools: {} },
        serverInfo: {
          name: "keylink-studio-gate-a-elicitation",
          version: "0.1.0",
        },
      });
      break;

    case "notifications/initialized":
      break;

    case "ping":
      sendResult(message.id, {});
      break;

    case "tools/list":
      sendResult(message.id, {
        tools: [
          {
            name: "gate_a_request_elicitation",
            description:
              "Open one harmless form elicitation for the Keylink Studio Gate A routing test.",
            inputSchema: {
              type: "object",
              properties: {},
              additionalProperties: false,
            },
          },
        ],
      });
      break;

    case "tools/call":
      if (message.params?.name !== "gate_a_request_elicitation") {
        sendError(message.id, -32602, "Unknown Gate A test tool.");
        break;
      }
      requestElicitation(message.id);
      break;

    default:
      if (message.id != null) {
        sendError(message.id, -32601, "Method not found.");
      }
  }
});
