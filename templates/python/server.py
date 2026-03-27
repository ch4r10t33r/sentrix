"""
Borgkit HTTP Server
──────────────────────────────────────────────────────────────────────────────
Starts an HTTP server for a WrappedAgent, exposing the standard Borgkit
endpoints so agents can discover and call each other over the network.

Endpoints
─────────
  POST /invoke         AgentRequest  → AgentResponse (JSON)
  POST /invoke/stream  AgentRequest  → Server-Sent Events (StreamChunk/StreamEnd)
  POST /gossip         GossipMessage (direct fan-out from peers)
  GET  /health         Heartbeat — lightweight, no auth
  GET  /anr            Full ANR (Agent Network Record) as JSON
  GET  /capabilities   Capability list as JSON

Usage
─────
  import asyncio
  from server import serve

  plugin = GoogleADKPlugin(config)
  agent  = plugin.wrap(my_adk_agent)
  asyncio.run(serve(agent, host="0.0.0.0", port=6174))

  # or via borgkit-cli:
  #   borgkit run MyAgent --port 6174
"""

from __future__ import annotations

import asyncio
import dataclasses
import json
import os
import signal
import time
import uuid
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from plugins.base import WrappedAgent


# ── CORS headers (needed for browser-based agent dashboards) ──────────────────

_CORS = {
    "Access-Control-Allow-Origin":  "*",
    "Access-Control-Allow-Headers": "Content-Type, Authorization, X-Payment, X-402-Payment",
    "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
}


# ── public entry point ────────────────────────────────────────────────────────

async def serve(
    agent:  "WrappedAgent",
    host:   str = "0.0.0.0",
    port:   int = 6174,
) -> None:
    """
    Start the HTTP transport for *agent* and block until shutdown.

    Order of operations:
      1. HTTP server starts listening (so peers can connect immediately
         after the ANR is announced)
      2. register_discovery() is called  →  prints startup banner
      3. Blocks on SIGINT / SIGTERM
      4. unregister_discovery() + graceful server shutdown

    Args:
        agent:  A WrappedAgent instance (from plugin.wrap(native_agent)).
        host:   Bind address.  Use "0.0.0.0" to accept connections from
                any interface (default); "127.0.0.1" for local-only.
        port:   TCP port.  Overridden by BORGKIT_PORT env var if set.
    """
    port = int(os.environ.get("BORGKIT_PORT", port))

    try:
        import aiohttp  # noqa: F401
        await _serve_aiohttp(agent, host, port)
    except ImportError:
        try:
            import uvicorn  # noqa: F401
            await _serve_uvicorn(agent, host, port)
        except ImportError:
            raise RuntimeError(
                "\n[Borgkit] No async HTTP backend found.\n"
                "Install one of:\n"
                "  pip install aiohttp\n"
                "  pip install fastapi uvicorn\n"
            )


# ── aiohttp implementation ────────────────────────────────────────────────────

async def _serve_aiohttp(agent: "WrappedAgent", host: str, port: int) -> None:
    from aiohttp import web

    app = web.Application()
    _register_aiohttp_routes(app, agent)

    runner = web.AppRunner(app, access_log=None)
    await runner.setup()
    site = web.TCPSite(runner, host, port)
    await site.start()

    # Register with discovery *after* the server is listening
    await agent.register_discovery()

    # ── wait for shutdown signal ──────────────────────────────────────────────
    loop   = asyncio.get_running_loop()
    stop   = loop.create_future()

    def _signal_handler() -> None:
        if not stop.done():
            stop.set_result(None)

    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            loop.add_signal_handler(sig, _signal_handler)
        except (NotImplementedError, OSError):
            # Windows does not support add_signal_handler for all signals
            pass

    try:
        await stop
    except asyncio.CancelledError:
        pass
    finally:
        print("\n[Borgkit] Shutting down gracefully…")
        try:
            await agent.unregister_discovery()
        except Exception:
            pass
        await runner.cleanup()


def _register_aiohttp_routes(app: Any, agent: "WrappedAgent") -> None:
    from aiohttp import web

    # ── POST /invoke ──────────────────────────────────────────────────────────
    async def handle_invoke(request: web.Request) -> web.Response:
        try:
            body = await request.json()
        except Exception:
            return web.json_response(
                {"error": "Request body must be valid JSON"},
                status=400, headers=_CORS,
            )

        from interfaces.agent_request import AgentRequest
        req = AgentRequest(
            request_id=body.get("requestId") or str(uuid.uuid4()),
            from_id   =body.get("from", "anonymous"),
            capability=body.get("capability", ""),
            payload   =body.get("payload", {}),
            signature =body.get("signature"),
            timestamp =body.get("timestamp"),
            x402      =body.get("x402"),
        )

        # x402 gate — check before invoking if the agent has pricing
        x402_status = _check_x402(agent, req)
        if x402_status:
            return web.json_response(x402_status, status=402, headers={**_CORS, **x402_status.get("_headers", {})})

        resp = await agent.handle_request(req)
        data = _serialise(resp)
        return web.json_response(data, headers=_CORS)

    # ── POST /gossip ──────────────────────────────────────────────────────────
    async def handle_gossip(request: web.Request) -> web.Response:
        try:
            body = await request.json()
        except Exception:
            return web.Response(status=400, headers=_CORS)

        try:
            from interfaces.iagent_mesh import GossipMessage
            msg = GossipMessage.from_dict(body)
            await agent.handle_gossip(msg)
        except Exception:
            pass  # gossip is best-effort; never crash the server
        return web.Response(status=204, headers=_CORS)

    # ── GET /health ───────────────────────────────────────────────────────────
    async def handle_health(request: web.Request) -> web.Response:
        from interfaces.iagent_mesh import HeartbeatRequest
        nonce = request.query.get("nonce", str(int(time.time() * 1_000)))
        hb_req = HeartbeatRequest(sender_id="__health__", nonce=nonce)
        hb_resp = await agent.handle_heartbeat(hb_req)
        return web.json_response(hb_resp.to_dict(), headers=_CORS)

    # ── GET /anr ──────────────────────────────────────────────────────────────
    async def handle_anr(request: web.Request) -> web.Response:
        entry = agent.get_anr()
        return web.json_response(_serialise(entry), headers=_CORS)

    # ── GET /capabilities ─────────────────────────────────────────────────────
    async def handle_capabilities(request: web.Request) -> web.Response:
        return web.json_response(
            {"agentId": agent.agent_id, "capabilities": agent.get_capabilities()},
            headers=_CORS,
        )

    # ── OPTIONS (CORS pre-flight) ─────────────────────────────────────────────
    async def handle_options(request: web.Request) -> web.Response:
        return web.Response(status=204, headers=_CORS)

    # ── POST /invoke/stream ───────────────────────────────────────────────────
    async def handle_invoke_stream(request: web.Request) -> web.StreamResponse:
        """
        Streaming endpoint — emits Server-Sent Events.

        Request body: same JSON shape as POST /invoke.
        Response:     text/event-stream; each event is a JSON-serialised
                      StreamChunk (type="chunk") or StreamEnd (type="end").

        Wire format per SSE event:
            data: {"type":"chunk","requestId":"…","delta":"…","sequence":N}\\n\\n
            data: {"type":"end","requestId":"…","finalResult":{…}}\\n\\n
        """
        try:
            body = await request.json()
        except Exception:
            return web.Response(
                text='data: {"type":"error","error":"Invalid JSON"}\\n\\n',
                status=400,
                content_type="text/event-stream",
                headers=_CORS,
            )

        from interfaces.agent_request import AgentRequest
        req = AgentRequest(
            request_id=body.get("requestId") or str(uuid.uuid4()),
            from_id   =body.get("from", "anonymous"),
            capability=body.get("capability", ""),
            payload   =body.get("payload", {}),
            signature =body.get("signature"),
            timestamp =body.get("timestamp"),
            x402      =body.get("x402"),
            stream    =True,
        )

        # x402 gate — same check as /invoke
        x402_status = _check_x402(agent, req)
        if x402_status:
            import json as _json
            err_event = f'data: {_json.dumps({"type": "error", "error": "Payment required", "x402": x402_status})}\\n\\n'
            return web.Response(
                text=err_event,
                status=402,
                content_type="text/event-stream",
                headers=_CORS,
            )

        # Start SSE response
        sse_response = web.StreamResponse(
            status=200,
            headers={
                **_CORS,
                "Content-Type":  "text/event-stream",
                "Cache-Control": "no-cache",
                "X-Accel-Buffering": "no",   # nginx: disable proxy buffering
            },
        )
        await sse_response.prepare(request)

        import json as _json
        try:
            async for event in agent.stream_request(req):
                frame = _json.dumps(event.to_dict(), ensure_ascii=False)
                await sse_response.write(f"data: {frame}\n\n".encode())
                if event.type == "end":
                    break
        except Exception as exc:
            from interfaces.iagent_mesh import StreamEnd
            end = StreamEnd(request_id=req.request_id, error=str(exc))
            frame = _json.dumps(end.to_dict(), ensure_ascii=False)
            try:
                await sse_response.write(f"data: {frame}\n\n".encode())
            except Exception:
                pass

        return sse_response

    app.router.add_post("/invoke",        handle_invoke)
    app.router.add_post("/invoke/stream", handle_invoke_stream)
    app.router.add_post("/gossip",        handle_gossip)
    app.router.add_get ("/health",        handle_health)
    app.router.add_get ("/anr",           handle_anr)
    app.router.add_get ("/capabilities",  handle_capabilities)
    app.router.add_route("OPTIONS", "/{path_info:.*}", handle_options)


# ── fastapi + uvicorn fallback ────────────────────────────────────────────────

async def _serve_uvicorn(agent: "WrappedAgent", host: str, port: int) -> None:
    from fastapi import FastAPI, Request, Response
    from fastapi.responses import JSONResponse
    import uvicorn

    app = FastAPI(
        title  =agent.config.name,
        version=agent.config.version,
        docs_url="/docs",
    )

    @app.post("/invoke")
    async def invoke(request: Request) -> JSONResponse:
        try:
            body = await request.json()
        except Exception:
            return JSONResponse({"error": "Invalid JSON"}, status_code=400, headers=_CORS)

        from interfaces.agent_request import AgentRequest
        req = AgentRequest(
            request_id=body.get("requestId") or str(uuid.uuid4()),
            from_id   =body.get("from", "anonymous"),
            capability=body.get("capability", ""),
            payload   =body.get("payload", {}),
            signature =body.get("signature"),
            timestamp =body.get("timestamp"),
            x402      =body.get("x402"),
        )
        x402_status = _check_x402(agent, req)
        if x402_status:
            return JSONResponse(x402_status, status_code=402, headers=_CORS)

        resp = await agent.handle_request(req)
        return JSONResponse(_serialise(resp), headers=_CORS)

    @app.post("/invoke/stream")
    async def invoke_stream(request: Request):
        """Streaming SSE endpoint (FastAPI/uvicorn fallback)."""
        from fastapi.responses import StreamingResponse
        import json as _json

        try:
            body = await request.json()
        except Exception:
            async def _err():
                yield 'data: {"type":"error","error":"Invalid JSON"}\n\n'
            return StreamingResponse(_err(), status_code=400, media_type="text/event-stream", headers=_CORS)

        from interfaces.agent_request import AgentRequest
        req = AgentRequest(
            request_id=body.get("requestId") or str(uuid.uuid4()),
            from_id   =body.get("from", "anonymous"),
            capability=body.get("capability", ""),
            payload   =body.get("payload", {}),
            signature =body.get("signature"),
            timestamp =body.get("timestamp"),
            x402      =body.get("x402"),
            stream    =True,
        )

        x402_status = _check_x402(agent, req)
        if x402_status:
            async def _x402():
                yield f'data: {_json.dumps({"type": "error", "error": "Payment required", "x402": x402_status})}\n\n'
            return StreamingResponse(_x402(), status_code=402, media_type="text/event-stream", headers=_CORS)

        async def _generate():
            try:
                async for event in agent.stream_request(req):
                    frame = _json.dumps(event.to_dict(), ensure_ascii=False)
                    yield f"data: {frame}\n\n"
                    if event.type == "end":
                        break
            except Exception as exc:
                from interfaces.iagent_mesh import StreamEnd
                end = StreamEnd(request_id=req.request_id, error=str(exc))
                yield f'data: {_json.dumps(end.to_dict(), ensure_ascii=False)}\n\n'

        return StreamingResponse(
            _generate(),
            media_type="text/event-stream",
            headers={**_CORS, "Cache-Control": "no-cache", "X-Accel-Buffering": "no"},
        )

    @app.post("/gossip", status_code=204)
    async def gossip(request: Request) -> Response:
        try:
            body = await request.json()
            from interfaces.iagent_mesh import GossipMessage
            await agent.handle_gossip(GossipMessage.from_dict(body))
        except Exception:
            pass
        return Response(status_code=204, headers=_CORS)

    @app.get("/health")
    async def health(nonce: str = "") -> JSONResponse:
        from interfaces.iagent_mesh import HeartbeatRequest
        nonce = nonce or str(int(time.time() * 1_000))
        hb_resp = await agent.handle_heartbeat(HeartbeatRequest(sender_id="__health__", nonce=nonce))
        return JSONResponse(hb_resp.to_dict(), headers=_CORS)

    @app.get("/anr")
    async def anr() -> JSONResponse:
        return JSONResponse(_serialise(agent.get_anr()), headers=_CORS)

    @app.get("/capabilities")
    async def capabilities() -> JSONResponse:
        return JSONResponse(
            {"agentId": agent.agent_id, "capabilities": agent.get_capabilities()},
            headers=_CORS,
        )

    # Register with discovery before starting uvicorn
    await agent.register_discovery()

    config = uvicorn.Config(
        app, host=host, port=port,
        loop="asyncio", log_level="warning",
        access_log=False,
    )
    server = uvicorn.Server(config)

    try:
        await server.serve()
    finally:
        try:
            await agent.unregister_discovery()
        except Exception:
            pass


# ── helpers ───────────────────────────────────────────────────────────────────

def _serialise(obj: Any) -> Any:
    """Convert a dataclass or object with to_dict() to a plain dict."""
    if hasattr(obj, "to_dict"):
        return obj.to_dict()
    if dataclasses.is_dataclass(obj) and not isinstance(obj, type):
        return dataclasses.asdict(obj)
    return obj


def _check_x402(agent: "WrappedAgent", req: Any) -> dict | None:
    """
    If the requested capability has x402 pricing configured and no valid
    payment proof is present, return a 402 response body.

    Returns None if the request may proceed.
    """
    pricing = getattr(agent.config, "x402_pricing", {}).get(req.capability)
    if not pricing:
        return None  # capability is free

    # If a payment proof is already attached, let it through
    if req.x402:
        return None

    # Build the 402 challenge body
    try:
        network = getattr(pricing, "network", "base")
        amount  = getattr(pricing, "amount_usd", 0)
        wallet  = getattr(pricing, "payee_address", "")
        return {
            "error":       "Payment required",
            "x402":        True,
            "capability":  req.capability,
            "price_usd":   amount,
            "network":     network,
            "payee":       wallet,
            "accepts": [
                {
                    "scheme":   "exact",
                    "network":  network,
                    "maxAmountRequired": str(int(amount * 1_000_000)),  # USDC 6-decimals
                    "resource": f"{agent.config.protocol}://{agent.config.host}:{agent.config.port}/invoke",
                    "asset":    "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",  # USDC on Base
                }
            ],
        }
    except Exception:
        return None
