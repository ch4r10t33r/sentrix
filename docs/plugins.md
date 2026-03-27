# Borgkit Plugins — Framework Adapters

Borgkit plugins let you bring **existing agents from any framework** into the mesh without rewriting them. A plugin is a thin translation layer that maps between a framework's native API and the Borgkit `IAgent` interface.

---

## Architecture

```
┌──────────────────────────────────┐
│     Framework-native agent        │
│  (LangGraph graph / ADK Agent)    │
└─────────────────┬────────────────┘
                  │  plugin.wrap(agent)
┌─────────────────▼────────────────┐
│        BorgkitPlugin              │
│  extractCapabilities()           │  ← inspects the native agent
│  translateRequest()              │  ← AgentRequest → native input
│  invokeNative()                  │  ← calls the native agent
│  translateResponse()             │  ← native output → AgentResponse
└─────────────────┬────────────────┘
                  │  returns IAgent
┌─────────────────▼────────────────┐
│         Borgkit mesh              │
│  ANR · Discovery · Wire format    │
└──────────────────────────────────┘
```

---

## BorgkitPlugin base class

```python
class BorgkitPlugin(ABC, Generic[TAgent]):

    @abstractmethod
    def extract_capabilities(self, agent: TAgent) -> list[CapabilityDescriptor]: ...

    @abstractmethod
    def translate_request(self, req: AgentRequest, cap: CapabilityDescriptor) -> Any: ...

    @abstractmethod
    def translate_response(self, result: Any, request_id: str) -> AgentResponse: ...

    @abstractmethod
    async def invoke_native(self, agent: TAgent, cap: CapabilityDescriptor, inp: Any) -> Any: ...

    def wrap(self, agent: TAgent) -> WrappedAgent: ...   # ← call this
```

Implement all four abstract methods and call `wrap()` to get back a fully Borgkit-compatible `IAgent`.

---

## LangGraph Plugin

Wraps any compiled LangGraph `CompiledGraph`.

### Capability extraction

The plugin discovers tools using four strategies (in priority order):

| Priority | Strategy | Condition |
|---|---|---|
| 1 | `capabilityMap` in config | explicit override |
| 2 | `graph.nodes["agent"].bound.tools` | standard ReAct pattern |
| 3 | `tools=` passed to plugin constructor | explicit tool list |
| 4 | Single `invoke` capability | whole-graph fallback |

### Usage

```python
from plugins.langgraph_plugin import wrap_langgraph
from langchain_core.tools import tool
from langchain_openai import ChatOpenAI
from langgraph.prebuilt import create_react_agent

@tool
def get_weather(city: str) -> str:
    """Get current weather for a city."""
    return f"22°C and sunny in {city}"

llm   = ChatOpenAI(model="gpt-4o-mini")
graph = create_react_agent(llm, tools=[get_weather])

agent = wrap_langgraph(
    graph    = graph,
    name     = "WeatherAgent",
    agent_id = "borgkit://agent/weather",
    tags     = ["weather"],
    tools    = [get_weather],   # explicit — fastest path
)

await agent.register_discovery()
```

### Config options (LangGraphPluginConfig)

| Option | Default | Description |
|---|---|---|
| `expose_tools_as_capabilities` | `True` | Each tool = one capability |
| `input_key` | `"messages"` | LangGraph state key for user input |
| `output_key` | `"messages"` | LangGraph state key for output |
| `agent_node_name` | `"agent"` | Node to inspect for tools |
| `recursion_limit` | `25` | LangGraph recursion limit |
| `stream` | `False` | Stream output instead of single invoke |

---

## Google ADK Plugin

Wraps any Google ADK `Agent` or `BaseAgent` subclass.

### Capability extraction

| Source | Attribute inspected |
|---|---|
| Primary | `agent.tools` / `agent._tools` |
| LlmAgent | `agent.canonical_tools` |
| Sub-agents | `agent.sub_agents` (opt-in via `expose_sub_agents=True`) |
| Schema | `tool.get_declaration()` → JSON Schema for input validation |

### Usage

```python
from plugins.google_adk_plugin import wrap_google_adk
from google.adk.agents import Agent
from google.adk.tools import FunctionTool

def search_docs(query: str) -> str:
    """Search the documentation knowledge base."""
    return f"Found 3 articles for '{query}'"

adk_agent = Agent(
    name        = "support_agent",
    model       = "gemini-2.0-flash",
    description = "Support agent",
    tools       = [FunctionTool(search_docs)],
)

agent = wrap_google_adk(
    agent    = adk_agent,
    name     = "SupportAgent",
    agent_id = "borgkit://agent/support",
    tags     = ["support", "helpdesk"],
)

await agent.register_discovery()
```

### Config options (GoogleADKPluginConfig)

| Option | Default | Description |
|---|---|---|
| `expose_tools_as_capabilities` | `True` | Each tool = one capability |
| `expose_sub_agents` | `False` | Sub-agents as extra capabilities |
| `async_mode` | `True` | Use `runner.run_async()` |
| `app_name` | `"borgkit"` | ADK Runner app name |
| `user_id` | `"borgkit-user"` | ADK session user ID |

---

## Writing a custom plugin

To integrate any other framework, subclass `BorgkitPlugin` and implement the four methods:

```python
from plugins.base import BorgkitPlugin, CapabilityDescriptor, PluginConfig
from interfaces import AgentRequest, AgentResponse

class MyFrameworkPlugin(BorgkitPlugin):

    def extract_capabilities(self, agent) -> list[CapabilityDescriptor]:
        # Inspect agent and return its capabilities
        return [
            CapabilityDescriptor(
                name        = "myCapability",
                description = "Does something useful",
                native_name = agent.some_internal_method.__name__,
            )
        ]

    def translate_request(self, req: AgentRequest, cap: CapabilityDescriptor):
        # Convert Borgkit AgentRequest → your framework's input format
        return {"input": req.payload.get("query"), "config": {}}

    async def invoke_native(self, agent, cap: CapabilityDescriptor, native_input):
        # Call your framework
        return await agent.run(native_input)

    def translate_response(self, result, request_id: str) -> AgentResponse:
        # Convert your framework's output → Borgkit AgentResponse
        return AgentResponse.success(request_id, {"output": str(result)})

# Usage
config = PluginConfig(agent_id="borgkit://agent/my", name="MyAgent", owner="0xWallet")
plugin = MyFrameworkPlugin(config)
agent  = plugin.wrap(my_framework_agent)
await agent.register_discovery()
```

---

## PluginConfig reference

All plugins share a common `PluginConfig` base:

| Field | Default | Description |
|---|---|---|
| `agent_id` | required | `"borgkit://agent/<name>"` |
| `owner` | required | Wallet / contract address |
| `name` | required | Human-readable agent name |
| `version` | `"0.1.0"` | Semantic version |
| `description` | `""` | Agent description |
| `tags` | `[]` | Discovery tags |
| `host` | `"localhost"` | Agent network host |
| `port` | `8080` | Agent API port |
| `protocol` | `"http"` | Transport protocol |
| `tls` | `False` | TLS flag |
| `discovery_type` | `"local"` | `"local"` \| `"http"` \| `"gossip"` |
| `discovery_url` | `None` | URL for HttpDiscovery |
| `signing_key` | `None` | 32-byte hex secp256k1 key for ANR signing |
| `capability_map` | `{}` | `{ "borgkitName": "nativeName" }` override |
