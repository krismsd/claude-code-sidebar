// Spike S5: minimal Claude Code channel (stdio MCP server, runs in WSL).
// On start it reports its parent PID + CC_SIDEBAR_TAG to the hook listener (so we can check
// correlation with the PID in hook headers), then pushes one hard-coded message into the session.
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { execSync } from "node:child_process";
import { readFileSync } from "node:fs";

const host = execSync("ip route show default | awk '{print $3; exit}'").toString().trim();
const report = (body) =>
  fetch(`http://${host}:47821/hook`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  }).catch(() => {});

// Walk up the process tree so we can see how we relate to the claude process.
const chain = [];
for (let pid = process.pid, i = 0; pid > 1 && i < 6; i++) {
  const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
  const ppid = Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
  const comm = stat.slice(stat.indexOf("(") + 1, stat.lastIndexOf(")"));
  chain.push({ pid, comm });
  pid = ppid;
}

const mcp = new Server(
  { name: "cc-sidebar", version: "0.0.1" },
  { capabilities: { experimental: { "claude/channel": {} } } },
);
await mcp.connect(new StdioServerTransport());

await report({
  hook_event_name: "ChannelHello",
  ppid: process.ppid,
  tag: process.env.CC_SIDEBAR_TAG ?? null,
  env_pid: process.env.CC_SIDEBAR_PID ?? null,
  chain,
});

setTimeout(async () => {
  await mcp.notification({
    method: "notifications/claude/channel",
    params: {
      content: "Hello from the cc-sidebar spike. Please reply with exactly: CHANNEL OK",
      meta: { sender: "cc_sidebar", kind: "prompt" },
    },
  });
  await report({ hook_event_name: "ChannelPushed" });
}, 15000);
