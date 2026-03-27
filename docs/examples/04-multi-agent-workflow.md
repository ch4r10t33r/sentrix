# Example 04 — Multi-Agent Workflow

This example shows how to chain multiple specialised agents into a coordinated workflow — a **pipeline** where each agent contributes one step and passes results to the next.

---

## Scenario: Travel Briefing Pipeline

Three agents collaborate to produce a complete travel briefing:

```
User
  │
  ▼
OrchestratorAgent  ─── discover & call ──>  WeatherAgent
                   ─── discover & call ──>  NewsAgent
                   ─── discover & call ──>  CurrencyAgent
                   ◄── aggregate results ──
  │
  ▼
"Travel briefing for Tokyo: 18°C, top story: ..., 1 USD = 149 JPY"
```

---

## Step 1 — Define the specialist agents

```python
# agents/weather_agent.py
from interfaces import IAgent, AgentRequest, AgentResponse

class WeatherAgent(IAgent):
    agent_id = 'borgkit://agent/weather'
    owner    = '0xWallet'
    def get_capabilities(self): return ['get_weather']
    async def handle_request(self, req):
        city = req.payload.get('city', 'London')
        return AgentResponse.success(req.request_id, {'temp': 18, 'condition': 'clear'})
```

```python
# agents/news_agent.py
class NewsAgent(IAgent):
    agent_id = 'borgkit://agent/news'
    owner    = '0xWallet'
    def get_capabilities(self): return ['get_headlines']
    async def handle_request(self, req):
        city = req.payload.get('city')
        return AgentResponse.success(req.request_id, {
            'headlines': [f'Top story in {city}: Markets at all-time high']
        })
```

```python
# agents/currency_agent.py
class CurrencyAgent(IAgent):
    agent_id = 'borgkit://agent/currency'
    owner    = '0xWallet'
    def get_capabilities(self): return ['get_exchange_rate']
    async def handle_request(self, req):
        target = req.payload.get('target', 'JPY')
        return AgentResponse.success(req.request_id, {'rate': 149.5, 'currency': target})
```

---

## Step 2 — Define the orchestrator

```python
# agents/orchestrator_agent.py
import asyncio, uuid
from interfaces import IAgent, AgentRequest, AgentResponse
from discovery.local_discovery import LocalDiscovery

class OrchestratorAgent(IAgent):
    agent_id = 'borgkit://agent/orchestrator'
    owner    = '0xWallet'
    metadata = {'name': 'OrchestratorAgent', 'version': '0.1.0'}

    def get_capabilities(self):
        return ['travel_briefing']

    async def handle_request(self, req: AgentRequest) -> AgentResponse:
        if req.capability == 'travel_briefing':
            city     = req.payload.get('city', 'Tokyo')
            currency = req.payload.get('currency', 'JPY')
            briefing = await self._build_briefing(city, currency)
            return AgentResponse.success(req.request_id, briefing)
        return AgentResponse.error(req.request_id, f'Unknown: {req.capability}')

    async def _build_briefing(self, city: str, currency: str) -> dict:
        registry = LocalDiscovery.get_instance()

        # ── fan out: all three calls in parallel ──────────────────────────────
        weather_task  = self._call(registry, 'get_weather',     {'city': city})
        news_task     = self._call(registry, 'get_headlines',   {'city': city})
        currency_task = self._call(registry, 'get_exchange_rate', {'target': currency})

        weather, news, fx = await asyncio.gather(
            weather_task, news_task, currency_task,
            return_exceptions=True,
        )

        # ── aggregate ─────────────────────────────────────────────────────────
        result = {'city': city}

        if isinstance(weather, dict):
            result['weather'] = f"{weather.get('temp')}°C, {weather.get('condition')}"
        else:
            result['weather'] = 'unavailable'

        if isinstance(news, dict):
            result['headlines'] = news.get('headlines', [])
        else:
            result['headlines'] = []

        if isinstance(fx, dict):
            result['exchange_rate'] = f"1 USD = {fx.get('rate')} {currency}"
        else:
            result['exchange_rate'] = 'unavailable'

        return result

    async def _call(self, registry, capability: str, payload: dict) -> dict:
        """Discover the first healthy peer for a capability and call it."""
        peers = await registry.query(capability)
        if not peers:
            raise RuntimeError(f'No agent found for capability: {capability}')

        peer = peers[0]

        # Resolve to live agent object stored in registry
        # (in production this would be an HTTP call)
        req = AgentRequest(
            request_id = str(uuid.uuid4()),
            from_id    = self.agent_id,
            capability = capability,
            payload    = payload,
        )

        # In-process call for this example
        agent_obj = _live_agents.get(peer.agent_id)
        if not agent_obj:
            raise RuntimeError(f'Agent {peer.agent_id} not reachable')

        resp = await agent_obj.handle_request(req)
        if resp.status == 'error':
            raise RuntimeError(resp.error_message)
        return resp.result
```

---

## Step 3 — Wire everything together

```python
# main.py
import asyncio, uuid
from discovery.local_discovery import LocalDiscovery
from interfaces.iagent_discovery import DiscoveryEntry, NetworkInfo, HealthStatus
from datetime import datetime, timezone

from agents.weather_agent      import WeatherAgent
from agents.news_agent         import NewsAgent
from agents.currency_agent     import CurrencyAgent
from agents.orchestrator_agent import OrchestratorAgent
from interfaces                import AgentRequest

# Keep a map of live agent objects for in-process calls
_live_agents = {}

async def register(agent, port: int):
    registry = LocalDiscovery.get_instance()
    entry = DiscoveryEntry(
        agent_id     = agent.agent_id,
        name         = type(agent).__name__,
        owner        = agent.owner,
        capabilities = agent.get_capabilities(),
        network      = NetworkInfo(protocol='http', host='localhost', port=port),
        health       = HealthStatus(
            status         = 'healthy',
            last_heartbeat = datetime.now(timezone.utc).isoformat(),
        ),
        registered_at = datetime.now(timezone.utc).isoformat(),
    )
    await registry.register(entry)
    _live_agents[agent.agent_id] = agent

async def main():
    # Register all specialist agents
    await register(WeatherAgent(),  port=8081)
    await register(NewsAgent(),     port=8082)
    await register(CurrencyAgent(), port=8083)
    await register(OrchestratorAgent(), port=8080)

    # Call the orchestrator
    req = AgentRequest(
        request_id = str(uuid.uuid4()),
        from_id    = '0xUser',
        capability = 'travel_briefing',
        payload    = {'city': 'Tokyo', 'currency': 'JPY'},
    )
    orchestrator = OrchestratorAgent()
    resp = await orchestrator.handle_request(req)

    print('=== Travel Briefing ===')
    for k, v in resp.result.items():
        print(f'  {k}: {v}')
    # === Travel Briefing ===
    #   city: Tokyo
    #   weather: 18°C, clear
    #   headlines: ['Top story in Tokyo: Markets at all-time high']
    #   exchange_rate: 1 USD = 149.5 JPY

asyncio.run(main())
```

---

## Pattern: parallel fan-out + sequential pipeline

The example above uses **parallel fan-out** (`asyncio.gather`). You can equally run steps **sequentially** when each step depends on the previous one:

```python
async def sequential_pipeline(city: str) -> dict:
    registry = LocalDiscovery.get_instance()

    # Step 1: get coordinates
    coords = await self._call(registry, 'geocode', {'city': city})

    # Step 2: use coords to get weather (sequential dependency)
    weather = await self._call(registry, 'get_weather_by_coords', coords)

    # Step 3: use weather to personalise news
    news = await self._call(registry, 'get_headlines', {
        'city': city, 'condition': weather.get('condition')
    })

    return {'coords': coords, 'weather': weather, 'news': news}
```

---

## Resilience patterns

```python
import asyncio

async def call_with_fallback(registry, capability, payload, timeout_s=5.0):
    """Try all healthy peers; return first successful response."""
    peers = await registry.query(capability)
    for peer in peers:
        try:
            agent = _live_agents[peer.agent_id]
            req   = AgentRequest(request_id=str(uuid.uuid4()),
                                  from_id='borgkit://agent/orchestrator',
                                  capability=capability, payload=payload)
            resp  = await asyncio.wait_for(agent.handle_request(req), timeout=timeout_s)
            if resp.status == 'success':
                return resp.result
        except (asyncio.TimeoutError, Exception):
            continue   # try next peer
    raise RuntimeError(f'All peers failed for capability: {capability}')
```
