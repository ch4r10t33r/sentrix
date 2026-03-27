# Example 02 — Making an Agent Discoverable

Once an agent is defined, it needs to **announce itself** so other agents can find it. This example shows the full registration lifecycle.

---

## Step 1 — Implement `register_discovery()`

The simplest approach: call `LocalDiscovery` directly.

```python
# agents/weather_agent.py
from interfaces import IAgent, AgentRequest, AgentResponse
from interfaces.iagent_discovery import DiscoveryEntry, NetworkInfo, HealthStatus
from discovery.local_discovery import LocalDiscovery
from datetime import datetime, timezone

class WeatherAgent(IAgent):
    agent_id = 'borgkit://agent/weather'
    owner    = '0xYourWalletAddress'
    metadata = {'name': 'WeatherAgent', 'version': '0.1.0', 'tags': ['weather']}

    def get_capabilities(self):
        return ['get_weather', 'get_forecast']

    async def handle_request(self, req: AgentRequest) -> AgentResponse:
        match req.capability:
            case 'get_weather':
                city = req.payload.get('city', 'London')
                return AgentResponse.success(req.request_id, {'temp': 22, 'city': city})
            case 'get_forecast':
                return AgentResponse.success(req.request_id, {'forecast': 'Sunny for 3 days'})
            case _:
                return AgentResponse.error(req.request_id, f'Unknown: {req.capability}')

    # ── Discovery ─────────────────────────────────────────────────────────────
    async def register_discovery(self) -> None:
        registry = LocalDiscovery.get_instance()
        await registry.register(DiscoveryEntry(
            agent_id     = self.agent_id,
            name         = 'WeatherAgent',
            owner        = self.owner,
            capabilities = self.get_capabilities(),
            network      = NetworkInfo(protocol='http', host='localhost', port=8080),
            health       = HealthStatus(
                status         = 'healthy',
                last_heartbeat = datetime.now(timezone.utc).isoformat(),
            ),
            registered_at = datetime.now(timezone.utc).isoformat(),
        ))
        print('[WeatherAgent] registered')

    async def unregister_discovery(self) -> None:
        await LocalDiscovery.get_instance().unregister(self.agent_id)
```

---

## Step 2 — Register on startup

```python
import asyncio
from agents.weather_agent import WeatherAgent

async def main():
    agent = WeatherAgent()
    await agent.register_discovery()

    print('Capabilities:', agent.get_capabilities())
    # → ['get_weather', 'get_forecast']

asyncio.run(main())
```

---

## Step 3 — Query the registry

Any other agent (or test) can now find WeatherAgent:

```python
from discovery.local_discovery import LocalDiscovery

async def find_weather_agents():
    registry = LocalDiscovery.get_instance()

    # Query by capability
    agents = await registry.query('get_weather')
    for a in agents:
        print(f'{a.name} @ {a.network.host}:{a.network.port}  caps={a.capabilities}')

    # List everything
    all_agents = await registry.list_all()
    print(f'Total agents on mesh: {len(all_agents)}')
```

---

## Step 4 — Sign an ANR record (optional but recommended)

For production, sign an ANR so peers can cryptographically verify your identity:

```python
import os
from anr.anr import AnrBuilder

private_key = bytes.fromhex(os.environ['AGENT_PRIVATE_KEY'])

anr_text = (
    AnrBuilder()
    .seq(1)
    .agent_id('borgkit://agent/weather')
    .name('WeatherAgent')
    .version('0.1.0')
    .capabilities(['get_weather', 'get_forecast'])
    .tags(['weather'])
    .proto('http')
    .agent_port(8080)
    .sign(private_key)
    .encode_text()
)

print(anr_text)
# → anr:enqFiW...

# Verify on the other side
from anr.anr import ANR
record = ANR.decode_text(anr_text)
assert record.verify()
parsed = record.parsed()
print(parsed.capabilities)  # ['get_weather', 'get_forecast']
```

---

## Step 5 — Switch to HttpDiscovery for production

Change **one env var** — no code changes required:

```bash
export BORGKIT_DISCOVERY_URL=https://registry.mycompany.com
python -m agents.weather_agent
```

The agent will automatically register with the remote registry and send heartbeats every 30 seconds.

---

## Heartbeats and health

Registered agents should send periodic heartbeats:

```python
import asyncio
from discovery.local_discovery import LocalDiscovery

async def heartbeat_loop(agent_id: str, interval_s: int = 30):
    registry = LocalDiscovery.get_instance()
    while True:
        await asyncio.sleep(interval_s)
        await registry.heartbeat(agent_id)

# Start alongside your agent
asyncio.create_task(heartbeat_loop('borgkit://agent/weather'))
```

Agents that stop heartbeating are marked `unhealthy` and excluded from `query()` results.
