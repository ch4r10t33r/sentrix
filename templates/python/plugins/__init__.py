"""
Borgkit Framework Plugins
────────────────────────
Lightweight adapter layers for integrating third-party agent frameworks
into the Borgkit network without rewriting agents.

Available plugins:
  Framework      Module                    Convenience fn
  ─────────────  ────────────────────────  ─────────────────────
  LangGraph      langgraph_plugin.py       wrap_langgraph()
  Google ADK     google_adk_plugin.py      wrap_google_adk()
  CrewAI         crewai_plugin.py          wrap_crewai()
  Agno           agno_plugin.py            wrap_agno()
  LlamaIndex     llamaindex_plugin.py      wrap_llamaindex()
  smolagents     smolagents_plugin.py      wrap_smolagents()

Adding a new framework:
  1. Create  plugins/my_framework_plugin.py
  2. Subclass BorgkitPlugin (base.py)
  3. Implement the four abstract methods:
       extract_capabilities(agent) -> List[CapabilityDescriptor]
       translate_request(req, descriptor) -> Any
       invoke_native(agent, descriptor, native_input) -> Any  (async)
       translate_response(native_result, request_id) -> AgentResponse
  4. Export a wrap_my_framework() convenience function

See base.py for the full interface contract.
"""

from .base import BorgkitPlugin, PluginConfig, CapabilityDescriptor, WrappedAgent

__all__ = [
    "BorgkitPlugin",
    "PluginConfig",
    "CapabilityDescriptor",
    "WrappedAgent",
]
