# Example 01 — Hello, Agent

This example shows the minimal code needed to define a Borgkit agent from scratch in each supported language.

---

## What we're building

A `GreetingAgent` that exposes one capability: `greet`. It takes a `name` in the payload and returns `"Hello, <name>!"`.

---

## TypeScript

```typescript
// agents/GreetingAgent.ts
import { IAgent, AgentMetadata } from '../interfaces/IAgent';
import { AgentRequest }          from '../interfaces/IAgentRequest';
import { AgentResponse }         from '../interfaces/IAgentResponse';

export class GreetingAgent implements IAgent {
  readonly agentId  = 'borgkit://agent/greeter';
  readonly owner    = '0xYourWalletAddress';
  readonly metadata: AgentMetadata = {
    name:    'GreetingAgent',
    version: '0.1.0',
    tags:    ['demo', 'greeting'],
  };

  getCapabilities(): string[] {
    return ['greet'];
  }

  async handleRequest(req: AgentRequest): Promise<AgentResponse> {
    if (req.capability === 'greet') {
      const name = (req.payload['name'] as string) ?? 'stranger';
      return {
        requestId: req.requestId,
        status:    'success',
        result:    { message: `Hello, ${name}!` },
        timestamp: Date.now(),
      };
    }
    return {
      requestId:    req.requestId,
      status:       'error',
      errorMessage: `Unknown capability: ${req.capability}`,
    };
  }
}

// --- run it ---
import { AgentRequest } from '../interfaces/IAgentRequest';

const agent = new GreetingAgent();
const req: AgentRequest = {
  requestId:  'req-001',
  from:       '0xCaller',
  capability: 'greet',
  payload:    { name: 'Alice' },
};
const resp = await agent.handleRequest(req);
console.log(resp.result); // { message: "Hello, Alice!" }
```

---

## Python

```python
# agents/greeting_agent.py
from interfaces import IAgent, AgentRequest, AgentResponse

class GreetingAgent(IAgent):
    agent_id = 'borgkit://agent/greeter'
    owner    = '0xYourWalletAddress'
    metadata = {'name': 'GreetingAgent', 'version': '0.1.0', 'tags': ['demo']}

    def get_capabilities(self):
        return ['greet']

    async def handle_request(self, req: AgentRequest) -> AgentResponse:
        if req.capability == 'greet':
            name = req.payload.get('name', 'stranger')
            return AgentResponse.success(req.request_id, {'message': f'Hello, {name}!'})
        return AgentResponse.error(req.request_id, f'Unknown capability: {req.capability}')


# --- run it ---
import asyncio

async def main():
    agent = GreetingAgent()
    req   = AgentRequest(request_id='req-001', from_id='0xCaller',
                         capability='greet', payload={'name': 'Alice'})
    resp  = await agent.handle_request(req)
    print(resp.result)  # {'message': 'Hello, Alice!'}

asyncio.run(main())
```

---

## Rust

```rust
// src/greeting_agent.rs
use crate::agent::IAgent;
use crate::request::AgentRequest;
use crate::response::AgentResponse;
use async_trait::async_trait;
use serde_json::json;

pub struct GreetingAgent;

#[async_trait]
impl IAgent for GreetingAgent {
    fn agent_id(&self) -> &str { "borgkit://agent/greeter" }
    fn owner(&self)    -> &str { "0xYourWalletAddress" }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["greet".into()]
    }

    async fn handle_request(&self, req: AgentRequest) -> AgentResponse {
        match req.capability.as_str() {
            "greet" => {
                let name = req.payload["name"].as_str().unwrap_or("stranger");
                AgentResponse::success(req.request_id, json!({ "message": format!("Hello, {}!", name) }))
            }
            _ => AgentResponse::error(req.request_id, format!("Unknown: {}", req.capability)),
        }
    }
}
```

---

## Zig

```zig
// src/greeting_agent.zig
const std   = @import("std");
const types = @import("interfaces/types.zig");

pub const GreetingAgent = struct {
    pub fn agentId(_: *const GreetingAgent) []const u8 { return "borgkit://agent/greeter"; }
    pub fn owner  (_: *const GreetingAgent) []const u8 { return "0xYourWalletAddress"; }

    pub fn getCapabilities(_: *const GreetingAgent) []const []const u8 {
        return &.{"greet"};
    }

    pub fn handleRequest(_: *const GreetingAgent, req: types.AgentRequest) types.AgentResponse {
        if (std.mem.eql(u8, req.capability, "greet")) {
            return types.AgentResponse.success(req.request_id, "Hello from GreetingAgent!");
        }
        return types.AgentResponse.err(req.request_id, "Unknown capability");
    }
};
```

---

## Scaffold with the CLI

Instead of writing the boilerplate manually:

```bash
borgkit init my-project --lang python
cd my-project
borgkit create agent GreetingAgent --capabilities greet
```

This generates the file, wires the capability, and drops it into `agents/`.
