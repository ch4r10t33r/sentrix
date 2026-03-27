# Example 03 — Agent-to-Agent Calls

This example shows how one Borgkit agent discovers and calls another, forming the basic building block of any multi-agent system.

---

## Scenario

- **WeatherAgent** — provides `get_weather` and `get_forecast`
- **ReportAgent** — discovers WeatherAgent at runtime and fetches data to build a report

Neither agent knows the other's address at compile time. ReportAgent uses the discovery layer to find WeatherAgent dynamically.

---

## The caller: ReportAgent

```python
# agents/report_agent.py
import uuid
import aiohttp
from interfaces import IAgent, AgentRequest, AgentResponse
from discovery.local_discovery import LocalDiscovery

class ReportAgent(IAgent):
    agent_id = 'borgkit://agent/reporter'
    owner    = '0xYourWalletAddress'
    metadata = {'name': 'ReportAgent', 'version': '0.1.0'}

    def get_capabilities(self):
        return ['build_weather_report']

    async def handle_request(self, req: AgentRequest) -> AgentResponse:
        if req.capability == 'build_weather_report':
            city = req.payload.get('city', 'London')
            report = await self._build_report(city)
            return AgentResponse.success(req.request_id, {'report': report})
        return AgentResponse.error(req.request_id, f'Unknown: {req.capability}')

    # ── core logic: discover → call → aggregate ────────────────────────────────
    async def _build_report(self, city: str) -> str:

        # 1. Discover agents that can provide weather data
        registry      = LocalDiscovery.get_instance()
        weather_peers = await registry.query('get_weather')

        if not weather_peers:
            return f'No weather agents available for {city}'

        # 2. Pick the first healthy peer
        peer = weather_peers[0]
        url  = f"http://{peer.network.host}:{peer.network.port}/invoke"

        # 3. Build a Borgkit AgentRequest
        req_payload = AgentRequest(
            request_id = str(uuid.uuid4()),
            from_id    = self.agent_id,
            capability = 'get_weather',
            payload    = {'city': city},
        )

        # 4. Call the peer over HTTP
        async with aiohttp.ClientSession() as session:
            async with session.post(url, json=req_payload.to_dict()) as resp:
                data = await resp.json()

        # 5. Parse and return
        temp = data.get('result', {}).get('temp', '?')
        return f'Weather report for {city}: {temp}°C'
```

---

## In-process shortcut (same process, no HTTP)

When both agents run in the same process, you can call directly without network overhead:

```python
# agents/report_agent_local.py
from discovery.local_discovery import LocalDiscovery
from interfaces import AgentRequest
import uuid

class ReportAgent(IAgent):
    # ... (same as above) ...

    async def _build_report_local(self, city: str) -> str:
        # Discover via local registry
        registry      = LocalDiscovery.get_instance()
        weather_peers = await registry.query('get_weather')

        if not weather_peers:
            return 'No weather agents'

        # Resolve the live agent object (stored alongside the entry)
        peer_entry = weather_peers[0]
        peer_agent = peer_entry.agent_ref   # set when registering in-process

        # Direct method call — no serialisation overhead
        req  = AgentRequest(
            request_id = str(uuid.uuid4()),
            from_id    = self.agent_id,
            capability = 'get_weather',
            payload    = {'city': city},
        )
        resp = await peer_agent.handle_request(req)
        return resp.result.get('temp', '?')
```

---

## Putting it together

```python
# main.py
import asyncio
from agents.weather_agent import WeatherAgent
from agents.report_agent  import ReportAgent
from interfaces import AgentRequest
import uuid

async def main():
    # Start agents
    weather = WeatherAgent()
    reporter = ReportAgent()

    await weather.register_discovery()
    await reporter.register_discovery()

    # Ask ReportAgent to build a report — it will discover WeatherAgent internally
    req = AgentRequest(
        request_id = str(uuid.uuid4()),
        from_id    = '0xUser',
        capability = 'build_weather_report',
        payload    = {'city': 'Tokyo'},
    )
    resp = await reporter.handle_request(req)
    print(resp.result['report'])
    # → "Weather report for Tokyo: 22°C"

asyncio.run(main())
```

---

## TypeScript equivalent

```typescript
import { AgentRequest }      from './interfaces/IAgentRequest';
import { LocalDiscovery }    from './discovery/LocalDiscovery';
import { WeatherAgent }      from './agents/WeatherAgent';
import { v4 as uuidv4 }     from 'uuid';

const registry = LocalDiscovery.getInstance();
const weather  = new WeatherAgent();
await weather.registerDiscovery?.();

// Another agent discovers WeatherAgent
const peers = await registry.query('getWeather');
if (peers.length === 0) throw new Error('No weather agents found');

const peer = peers[0];
console.log(`Found: ${peer.name} @ ${peer.network.host}:${peer.network.port}`);

// Call it
const req: AgentRequest = {
  requestId:  uuidv4(),
  from:       'borgkit://agent/caller',
  capability: 'getWeather',
  payload:    { city: 'London' },
};
const resp = await weather.handleRequest(req);
console.log(resp.result);  // { temp: 22, city: "London" }
```

---

## Error handling best practices

```python
async def safe_call(peer_agent, capability: str, payload: dict) -> dict | None:
    req = AgentRequest(
        request_id = str(uuid.uuid4()),
        from_id    = 'borgkit://agent/caller',
        capability = capability,
        payload    = payload,
    )
    try:
        resp = await asyncio.wait_for(
            peer_agent.handle_request(req),
            timeout=10.0,           # always set a timeout
        )
        if resp.status == 'error':
            print(f'Agent returned error: {resp.error_message}')
            return None
        return resp.result
    except asyncio.TimeoutError:
        print(f'Agent timed out on {capability}')
        return None
    except Exception as e:
        print(f'Call failed: {e}')
        return None
```
