"""
Google ADK → Borgkit Plugin
──────────────────────────────────────────────────────────────────────────────
Wraps a Google Agent Development Kit `Agent` (or `BaseAgent` subclass) so it
appears as a standard Borgkit IAgent on the mesh.

Capability extraction strategy
──────────────────────────────
  1. Explicit capability_map in PluginConfig
  2. Agent.tools list  (FunctionTool / BaseTool instances)
  3. Sub-agents in a multi-agent setup  (Agent.sub_agents)
  4. Single-capability fallback: one capability = one agent invocation

Invocation modes
───────────────
  - run_async (default)  : uses ADK Runner.run_async()
  - sync                 : wraps Runner.run() in a threadpool

Usage
─────
  from plugins.google_adk_plugin import GoogleADKPlugin, GoogleADKPluginConfig

  config = GoogleADKPluginConfig(
      agent_id = "borgkit://agent/support",
      name     = "SupportAgent",
      version  = "1.0.0",
      tags     = ["support", "adk"],
  )
  plugin = GoogleADKPlugin(config)
  agent  = plugin.wrap(my_adk_agent)

  await agent.register_discovery()

Install deps:
  pip install google-adk
"""

from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass, field
from typing import Any, Optional

from interfaces import AgentRequest, AgentResponse
from plugins.base import BorgkitPlugin, CapabilityDescriptor, PluginConfig


# ── extended config ───────────────────────────────────────────────────────────

@dataclass
class GoogleADKPluginConfig(PluginConfig):
    """Google ADK-specific configuration (extends PluginConfig)."""

    # Session/user IDs used when creating an ADK Runner session
    app_name:   str = "borgkit"
    user_id:    str = "borgkit-user"

    # If True, sub-agents are exposed as capabilities too
    expose_sub_agents: bool = False

    # If True, each tool is exposed as a separate Borgkit capability
    expose_tools_as_capabilities: bool = True

    # Key in ADK content that holds the output text (default: first part)
    output_content_key: Optional[str] = None

    # Async mode (recommended) vs sync
    async_mode: bool = True


# ── plugin implementation ─────────────────────────────────────────────────────

class GoogleADKPlugin(BorgkitPlugin):
    """
    Borgkit plugin for Google ADK agents.
    Wraps any `google.adk.agents.Agent` or `BaseAgent` subclass.
    """

    def __init__(self, config: GoogleADKPluginConfig):
        super().__init__(config)
        self._adk_config = config

    # ── capability extraction ─────────────────────────────────────────────────

    def extract_capabilities(self, agent: Any) -> list[CapabilityDescriptor]:
        caps: list[CapabilityDescriptor] = []

        if self._adk_config.expose_tools_as_capabilities:
            tools = self._get_tools(agent)
            caps.extend(self._tool_to_descriptor(t) for t in tools)

        if self._adk_config.expose_sub_agents:
            subs = getattr(agent, 'sub_agents', None) or []
            for sub in subs:
                caps.append(CapabilityDescriptor(
                    name        = self._sanitize(getattr(sub, 'name', 'subagent')),
                    description = getattr(sub, 'description', ''),
                    native_name = getattr(sub, 'name', ''),
                    tags        = ['sub-agent'],
                ))

        if not caps:
            # Single-invocation fallback
            caps.append(CapabilityDescriptor(
                name        = "invoke",
                description = getattr(agent, 'description', '') or self._adk_config.description,
                native_name = "__agent__",
                tags        = self._adk_config.tags,
            ))

        return caps

    @staticmethod
    def _get_tools(agent: Any) -> list:
        """
        Extract tools from an ADK agent.
        Handles: .tools list, ._tools, and LlmAgent.canonical_tools.
        """
        for attr in ('tools', '_tools', 'canonical_tools'):
            tools = getattr(agent, attr, None)
            if tools:
                return list(tools)
        return []

    @staticmethod
    def _tool_to_descriptor(tool: Any) -> CapabilityDescriptor:
        name = getattr(tool, 'name', None) or getattr(tool, '__name__', str(tool))
        desc = getattr(tool, 'description', '')

        # ADK FunctionTool wraps a Python function — extract its schema
        input_schema = None
        try:
            fn = getattr(tool, '_func', None) or getattr(tool, 'func', None)
            if fn and hasattr(fn, '__annotations__'):
                input_schema = {
                    "type": "object",
                    "properties": {
                        k: {"type": "string"}
                        for k in fn.__annotations__
                        if k != "return"
                    }
                }
        except Exception:
            pass

        # Also try ADK's built-in schema
        if not input_schema:
            try:
                schema_fn = getattr(tool, 'get_declaration', None)
                if schema_fn:
                    decl = schema_fn()
                    params = getattr(decl, 'parameters', None)
                    if params:
                        input_schema = {"type": "object", "properties": dict(params)}
            except Exception:
                pass

        return CapabilityDescriptor(
            name=GoogleADKPlugin._sanitize(name),
            description=desc,
            native_name=name,
            input_schema=input_schema,
        )

    @staticmethod
    def _sanitize(name: str) -> str:
        """Normalize tool names to valid Borgkit capability names."""
        return name.replace(' ', '_').replace('-', '_').lower()

    # ── request translation ───────────────────────────────────────────────────

    def translate_request(
        self,
        req: AgentRequest,
        descriptor: CapabilityDescriptor,
    ) -> dict:
        """
        Build the ADK-compatible invocation dict from a Borgkit AgentRequest.
        Returns a dict with:
          - message   : the text turn to send to the agent
          - tool_name : native tool name (if routing to a specific tool)
          - payload   : original payload for reference
        """
        native = (
            self.config.capability_map.get(req.capability)
            or descriptor.native_name
            or req.capability
        )

        # Construct a natural language turn if payload is simple
        if native == "__agent__":
            text = (
                req.payload.get("message")
                or req.payload.get("input")
                or req.payload.get("query")
                or json.dumps(req.payload)
            )
        else:
            # Invoke a specific tool by name: craft a prompt that includes args
            args_str = json.dumps(req.payload, indent=2)
            text = f"Call the tool `{native}` with these arguments:\n{args_str}"

        return {
            "message":   text,
            "tool_name": native,
            "payload":   req.payload,
            "from_id":   req.from_id,
        }

    # ── response translation ──────────────────────────────────────────────────

    def translate_response(self, native_result: Any, request_id: str) -> AgentResponse:
        """
        Convert ADK's response into a Borgkit AgentResponse.
        Handles ADK's Event / RunResult / string responses.
        """
        try:
            content = self._extract_adk_content(native_result)
            return AgentResponse.success(request_id, {
                "content": content,
                "raw":     self._serialize(native_result),
            })
        except Exception as e:
            return AgentResponse.error(request_id, f"Response translation failed: {e}")

    def _extract_adk_content(self, result: Any) -> str:
        # String shortcut
        if isinstance(result, str):
            return result

        # ADK RunResult / Event list
        if isinstance(result, list):
            parts = []
            for event in result:
                text = self._event_text(event)
                if text:
                    parts.append(text)
            return "\n".join(parts) if parts else str(result)

        # Single event / RunResult
        text = self._event_text(result)
        if text:
            return text

        # Try .response or .content attributes
        for attr in ('response', 'content', 'text', 'output'):
            val = getattr(result, attr, None)
            if val is not None:
                return str(val)

        return str(result)

    @staticmethod
    def _event_text(event: Any) -> Optional[str]:
        """Pull text from an ADK Event object."""
        # ADK Event has .content -> Content -> .parts -> [Part]
        content = getattr(event, 'content', None)
        if content is None:
            return None
        parts = getattr(content, 'parts', None)
        if not parts:
            return None
        texts = []
        for part in parts:
            t = getattr(part, 'text', None)
            if t:
                texts.append(t)
        return " ".join(texts) if texts else None

    @staticmethod
    def _serialize(obj: Any) -> Any:
        try:
            return json.loads(json.dumps(obj, default=str))
        except Exception:
            return str(obj)

    # ── native invocation ─────────────────────────────────────────────────────

    async def invoke_native(
        self,
        agent: Any,
        descriptor: CapabilityDescriptor,
        native_input: dict,
    ) -> Any:
        """
        Call the ADK agent through its Runner interface.
        Creates a fresh in-memory session per request.
        """
        try:
            from google.adk.runners import Runner
            from google.adk.sessions import InMemorySessionService
            from google.genai import types as genai_types
        except ImportError as e:
            raise RuntimeError(
                f"google-adk not installed or import failed: {e}\n"
                "Install with: pip install google-adk"
            ) from e

        session_svc = InMemorySessionService()
        runner      = Runner(
            agent           = agent,
            app_name        = self._adk_config.app_name,
            session_service = session_svc,
        )

        session = await session_svc.create_session(
            app_name=self._adk_config.app_name,
            user_id=self._adk_config.user_id,
        )

        message  = native_input["message"]
        user_msg = genai_types.Content(
            role  = "user",
            parts = [genai_types.Part(text=message)],
        )

        events = []
        if self._adk_config.async_mode:
            async for event in runner.run_async(
                user_id=self._adk_config.user_id,
                session_id=session.id,
                new_message=user_msg,
            ):
                events.append(event)
        else:
            events = await asyncio.get_event_loop().run_in_executor(
                None,
                lambda: list(runner.run(
                    user_id=self._adk_config.user_id,
                    session_id=session.id,
                    new_message=user_msg,
                ))
            )

        return events


# ── convenience function ──────────────────────────────────────────────────────

def wrap_google_adk(
    agent:    Any,
    name:     str,
    agent_id: str,
    owner:    str  = "0xYourWalletAddress",
    version:  str  = "0.1.0",
    tags:     list[str] | None = None,
    expose_tools: bool = True,
    **kwargs,
) -> Any:
    """
    One-liner helper to wrap a Google ADK agent.

    Example:
      from plugins.google_adk_plugin import wrap_google_adk
      from google.adk.agents import Agent

      adk_agent = Agent(name="support", model="gemini-2.0-flash", tools=[...])
      borgkit_agent = wrap_google_adk(
          agent    = adk_agent,
          name     = "SupportAgent",
          agent_id = "borgkit://agent/support",
          tags     = ["support", "helpdesk"],
      )
      await borgkit_agent.register_discovery()
    """
    config = GoogleADKPluginConfig(
        agent_id=agent_id, name=name, owner=owner,
        version=version, tags=tags or [],
        expose_tools_as_capabilities=expose_tools,
        **kwargs,
    )
    return GoogleADKPlugin(config).wrap(agent)
