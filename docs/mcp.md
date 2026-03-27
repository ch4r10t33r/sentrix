# Borgkit ↔ MCP Bridge

Borgkit has a two-way bridge with the [Model Context Protocol (MCP)](https://modelcontextprotocol.io):

| Direction | What it means | How |
|---|---|---|
| **MCP → Borgkit** | Any MCP server becomes a Borgkit agent | `MCPPlugin` |
| **Borgkit → MCP** | Any Borgkit agent becomes an MCP server | `serve_as_mcp()` / `serveAsMcp()` |

This makes Borgkit the interoperability hub between MCP's vast tool ecosystem (GitHub, filesystem, Slack, databases, web search…) and every agent framework Borgkit supports (Google ADK, CrewAI, LangGraph, Agno, smolagents, and more).

---

## Direction 1 — MCP server → Borgkit agent

Wrap any MCP server so its tools appear as Borgkit capabilities. The resulting agent can be registered with discovery, called by other Borgkit agents, and served over HTTP.

### Python

```python
import asyncio
from plugins.mcp_plugin import MCPPlugin
from plugins.base import PluginConfig

config = PluginConfig(
    agent_id="borgkit://agent/github-mcp",
    name="GitHubMCP",
    owner="0xYourWallet",
    port=8081,
    discovery_type="local",
)

async def main():
    # ── Stdio (subprocess) ────────────────────────────────────────────────────
    plugin = await MCPPlugin.from_command(
        command=["npx", "-y", "@modelcontextprotocol/server-github"],
        config=config,
        env={"GITHUB_TOKEN": "ghp_..."},
    )

    # ── SSE / HTTP ────────────────────────────────────────────────────────────
    # plugin = await MCPPlugin.from_url(
    #     url="http://localhost:3000/sse",
    #     config=config,
    #     headers={"Authorization": "Bearer sk-..."},
    # )

    agent = plugin.wrap()
    print("Capabilities:", agent.get_capabilities())
    # → ['create_or_update_file', 'search_repositories', 'create_issue', ...]

    await agent.serve(port=8081)       # starts HTTP server + registers with discovery
    await plugin.close()               # called automatically on SIGINT

asyncio.run(main())
```

### TypeScript

```ts
import { MCPPlugin }  from './plugins/MCPPlugin';
import { PluginConfig } from './plugins/IPlugin';

const config: PluginConfig = {
  agentId: 'borgkit://agent/github-mcp',
  name:    'GitHubMCP',
  owner:   '0xYourWallet',
  port:    8081,
};

// Stdio (subprocess)
const plugin = await MCPPlugin.fromCommand(
  ['npx', '-y', '@modelcontextprotocol/server-github'],
  config,
  { GITHUB_TOKEN: 'ghp_...' },
);

// SSE / HTTP
// const plugin = await MCPPlugin.fromUrl('http://localhost:3000/sse', config);

const agent = plugin.wrap();
console.log('Capabilities:', agent.getCapabilities());

await agent.serve({ port: 8081 });
await plugin.close();
```

### Supported transports (inbound)

| Transport | Factory method | When to use |
|---|---|---|
| **Stdio** | `MCPPlugin.from_command(["cmd", ...])` | Any MCP server you can launch as a process |
| **SSE** | `MCPPlugin.from_url("http://host/sse")` | MCP servers with HTTP/SSE endpoints |
| **Streamable HTTP** | `MCPPlugin.from_url("http://host/mcp")` | MCP spec 2025-03-26+; auto-detected |

---

## Direction 2 — Borgkit agent → MCP server

Expose any Borgkit agent so MCP clients (Claude Desktop, Cursor, Continue…) can call its capabilities as native MCP tools.

### Python

```python
import asyncio
from adapters.mcp_server import serve_as_mcp
from agents.my_agent import MyAgent

agent = MyAgent()

# stdio — for Claude Desktop / Cursor
asyncio.run(serve_as_mcp(agent))

# SSE — for remote / multi-client access
asyncio.run(serve_as_mcp(agent, transport="sse", port=3000))
```

### TypeScript

```ts
import { serveAsMcp } from './adapters/MCPServer';
import { MyAgent }    from './agents/MyAgent';

const agent = new MyAgent();

// stdio — for Claude Desktop / Cursor
await serveAsMcp(agent);

// SSE — for remote / multi-client access
await serveAsMcp(agent, { transport: 'sse', port: 3000 });
```

### Connecting Claude Desktop

Add to `~/.config/claude/claude_desktop_config.json` (macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "my-borgkit-agent": {
      "command": "python",
      "args": ["path/to/run_mcp.py"]
    }
  }
}
```

`run_mcp.py`:
```python
import asyncio
from agents.my_agent import MyAgent
from adapters.mcp_server import serve_as_mcp

asyncio.run(serve_as_mcp(MyAgent()))
```

### Supported transports (outbound)

| Transport | `transport=` | Endpoint | Use case |
|---|---|---|---|
| **stdio** | `"stdio"` (default) | stdin/stdout | Claude Desktop, Cursor, Continue |
| **SSE** | `"sse"` | `GET /sse`, `POST /messages` | Remote clients, browser tooling |
| **Streamable HTTP** | `"http"` | `POST /mcp` | MCP spec 2025-03-26+, REST clients |

---

## Using MCP tools in a multi-agent workflow

The MCP bridge composes naturally with the rest of Borgkit. Here's a complete example: a research agent that uses GitHub + filesystem MCP tools, publishing its results via a Borgkit capability that another agent can call.

```python
import asyncio
from plugins.mcp_plugin   import MCPPlugin
from plugins.base         import PluginConfig
from interfaces           import AgentRequest
from interfaces.iagent_client import AgentClient
from discovery.local_discovery import LocalDiscovery

async def main():
    # ── Wrap the GitHub MCP server as a Borgkit agent ─────────────────────────
    github_plugin = await MCPPlugin.from_command(
        ["npx", "-y", "@modelcontextprotocol/server-github"],
        PluginConfig(
            agent_id="borgkit://agent/github",
            name="GitHub",
            owner="0xBot",
            port=8082,
        ),
        env={"GITHUB_TOKEN": "ghp_..."},
    )
    github_agent = github_plugin.wrap()
    await github_agent.register_discovery()

    # ── Another agent discovers and calls it ──────────────────────────────────
    client = AgentClient(discovery=LocalDiscovery.get_instance())

    entry = await client.find("search_repositories")
    if entry:
        resp = await client.call_entry(
            entry,
            capability="search_repositories",
            payload={"query": "borgkit agent protocol"},
        )
        print(resp.result)

    await github_plugin.close()

asyncio.run(main())
```

---

## x402 payments on MCP capabilities

If an MCP-wrapped capability has x402 pricing configured, the Borgkit HTTP server automatically gates it — the MCP client receives a clear payment-required error with the requirements.

```python
from addons.x402.types import CapabilityPricing

config = PluginConfig(
    ...
    x402_pricing={
        "premium_search": CapabilityPricing.usdc_base(0.001, "0xYourWallet"),
    },
)
```

When a caller hits `premium_search` without a payment proof, the server returns:
```json
{
  "status": "payment_required",
  "x402":   true,
  "capability": "premium_search",
  "price_usd":  0.001
}
```

---

## Installation

### Python
```bash
pip install mcp
# already included in templates/python/requirements.txt
```

### TypeScript
```bash
npm install @modelcontextprotocol/sdk
# already included in templates/typescript/package.json.tpl
```
