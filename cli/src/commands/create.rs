use anyhow::{bail, Result};
use clap::Args;
use std::io::{self, Write as IoWrite};
use std::path::Path;

use crate::detect_lang::{self, Lang};
use crate::logger;

// ── Args ──────────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct CreateArgs {
    /// Subcommand: "agent"
    pub subcommand: String,
    /// Agent name
    pub name: Option<String>,
    #[arg(short, long)]
    pub lang: Option<String>,
    #[arg(short, long, default_value = "")]
    pub capabilities: String,
    #[arg(short, long, default_value = "none")]
    pub framework: String,
    #[arg(long)]
    pub addon: Option<String>,
    #[arg(short, long)]
    pub yes: bool,
}

// ── Framework enum ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Framework {
    None,
    GoogleAdk,
    CrewAi,
    LangGraph,
    Agno,
    LlamaIndex,
    Smolagents,
}

impl Framework {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().replace(['-', '_'], "").as_str() {
            "googleadk" => Self::GoogleAdk,
            "crewai" => Self::CrewAi,
            "langgraph" => Self::LangGraph,
            "agno" => Self::Agno,
            "llamaindex" => Self::LlamaIndex,
            "smolagents" => Self::Smolagents,
            _ => Self::None,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::GoogleAdk => "google-adk",
            Self::CrewAi => "crewai",
            Self::LangGraph => "langgraph",
            Self::Agno => "agno",
            Self::LlamaIndex => "llamaindex",
            Self::Smolagents => "smolagents",
        }
    }

    /// Install hint for a given language, if any.
    fn install_hint(&self, lang: &Lang) -> Option<&'static str> {
        match (self, lang) {
            (Self::GoogleAdk, Lang::TypeScript) => {
                Some("npm install @google/generative-ai @google-labs/agent-development-kit")
            }
            (Self::GoogleAdk, Lang::Python) => Some("pip install google-adk google-generativeai"),
            (Self::CrewAi, Lang::Python) => Some("pip install crewai crewai-tools"),
            (Self::LangGraph, Lang::TypeScript) => {
                Some("npm install @langchain/langgraph @langchain/openai @langchain/core zod")
            }
            (Self::LangGraph, Lang::Python) => {
                Some("pip install langgraph langchain-openai langchain-core")
            }
            (Self::Agno, Lang::Python) => Some("pip install agno openai"),
            (Self::LlamaIndex, Lang::TypeScript) => {
                Some("npm install llamaindex @llamaindex/openai")
            }
            (Self::LlamaIndex, Lang::Python) => {
                Some("pip install llama-index llama-index-llms-openai")
            }
            (Self::Smolagents, Lang::Python) => Some("pip install smolagents"),
            _ => None,
        }
    }

    fn is_python_only(&self) -> bool {
        matches!(self, Self::CrewAi | Self::Agno | Self::Smolagents)
    }
}

// ── Case helpers ──────────────────────────────────────────────────────────────

fn pascal_case(s: &str) -> String {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut chars = p.chars();
            match chars.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

/// Convert PascalCase or camelCase string to snake_case.
fn snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

// ── Capability formatters ─────────────────────────────────────────────────────

/// `["echo", "ping"]`  — TypeScript array literal
fn caps_as_ts(caps: &[String]) -> String {
    let inner: Vec<String> = caps.iter().map(|c| format!("'{c}'")).collect();
    format!("[{}]", inner.join(", "))
}

/// `vec!["echo", "ping"]`  — Rust macro
fn caps_as_rs(caps: &[String]) -> String {
    let inner: Vec<String> = caps
        .iter()
        .map(|c| format!(r#""{c}".to_string()"#))
        .collect();
    format!("vec![{}]", inner.join(", "))
}

/// `&[_]{ "echo", "ping" }`  — Zig array
fn caps_as_zig(caps: &[String]) -> String {
    let inner: Vec<String> = caps.iter().map(|c| format!(r#""{c}""#)).collect();
    format!("&.{{ {} }}", inner.join(", "))
}

// ── install_deps ──────────────────────────────────────────────────────────────

fn install_deps(lang: &Lang, dir: &Path) {
    let (prog, args): (&str, Vec<&str>) = match lang {
        Lang::TypeScript => ("npm", vec!["install"]),
        Lang::Python => ("pip", vec!["install", "-r", "requirements.txt"]),
        Lang::Rust => ("cargo", vec!["build"]),
        Lang::Zig => ("zig", vec!["build"]),
    };

    let status = std::process::Command::new(prog)
        .args(&args)
        .current_dir(dir)
        .status();

    match status {
        Ok(s) if s.success() => logger::success(&format!("Dependencies installed ({prog})")),
        Ok(s) => logger::warn(&format!("`{prog}` exited with {s}")),
        Err(e) => logger::warn(&format!("Could not run `{prog}`: {e}")),
    }
}

// ── Template: TypeScript × none ───────────────────────────────────────────────

fn template_ts_none(name: &str, caps: &[String]) -> String {
    let name_lower = name.to_lowercase();
    let _cap_tags = caps_as_ts(caps);
    let cap_list = {
        let inner: Vec<String> = caps.iter().map(|c| format!("'{c}'")).collect();
        inner.join(", ")
    };
    let switch_cases: String = caps.iter().map(|c| {
        format!(
            "      case '{c}':\n        // TODO: implement {c}\n        return {{ requestId: req.requestId, status: 'success', result: {{ message: '{c} called' }} }};"
        )
    }).collect::<Vec<_>>().join("\n");

    format!(
        r#"import {{ IAgent }}        from '../interfaces/IAgent';
import {{ AgentRequest }}  from '../interfaces/IAgentRequest';
import {{ AgentResponse }} from '../interfaces/IAgentResponse';

export class {name} implements IAgent {{
  readonly agentId  = 'sentrix://agent/{name_lower}';
  readonly owner    = '0xYourWalletAddress';
  readonly metadata = {{ name: '{name}', version: '0.1.0', tags: [{cap_list}] }};

  getCapabilities(): string[] {{
    return [{cap_list}];
  }}

  async handleRequest(req: AgentRequest): Promise<AgentResponse> {{
    switch (req.capability) {{
{switch_cases}
      default:
        return {{ requestId: req.requestId, status: 'error', errorMessage: `Unknown capability: ${{req.capability}}` }};
    }}
  }}

  async registerDiscovery(): Promise<void> {{
    // TODO: plug in your discovery adapter
    console.log('[{name}] registered with discovery layer');
  }}
}}
"#
    )
}

// ── Template: TypeScript × google-adk ────────────────────────────────────────

fn template_ts_google_adk(name: &str, caps: &[String]) -> String {
    let name_lower = name.to_lowercase();
    let cap_list_sq: Vec<String> = caps.iter().map(|c| format!("'{c}'")).collect();
    let cap_list = cap_list_sq.join(", ");
    let tags_with_adk = {
        let mut v = cap_list_sq.clone();
        v.push("'adk'".to_string());
        v.join(", ")
    };

    let fn_defs: String = caps.iter().map(|c| {
        format!(
            "\nfunction {c}({{ query }}: {{ query: string }}): string {{\n  // TODO: implement {c}\n  return `{c} result for: ${{query}}`;\n}}"
        )
    }).collect::<Vec<_>>().join("\n");

    let function_tools: Vec<String> = caps
        .iter()
        .map(|c| format!("new FunctionTool({c})"))
        .collect();
    let function_tools_str = function_tools.join(", ");

    format!(
        r#"/**
 * {name} — Google ADK agent, wrapped for the Sentrix mesh.
 *
 * Install: npm install @google-labs/agent-development-kit @google/generative-ai
 */
import {{ Agent, FunctionTool }}  from '@google-labs/agent-development-kit';
import {{ IAgent }}               from '../interfaces/IAgent';
import {{ AgentRequest }}         from '../interfaces/IAgentRequest';
import {{ AgentResponse }}        from '../interfaces/IAgentResponse';

// ── Define one function per capability ────────────────────────────────────────
{fn_defs}

// ── Build the Google ADK agent ────────────────────────────────────────────────
const _adkAgent = new Agent({{
  name:        '{name_lower}',
  model:       'gemini-2.0-flash',
  description: '{name} — built with Google ADK',
  tools:       [{function_tools_str}],
}});

// ── Sentrix-compliant wrapper ─────────────────────────────────────────────────
// Adapts the ADK agent to the IAgent interface for full mesh interoperability.
export const {name}: IAgent = {{
  agentId:  'sentrix://agent/{name_lower}',
  owner:    '0xYourWalletAddress',
  metadata: {{ name: '{name}', version: '0.1.0', tags: [{tags_with_adk}] }},

  getCapabilities: () => [{cap_list}],

  async handleRequest(req: AgentRequest): Promise<AgentResponse> {{
    try {{
      // Route to the ADK agent via its native runner
      const runner = _adkAgent.createRunner();
      const result = await runner.run({{ input: JSON.stringify(req.payload) }});
      return {{ requestId: req.requestId, status: 'success', result: {{ content: String(result) }} }};
    }} catch (err: any) {{
      return {{ requestId: req.requestId, status: 'error', errorMessage: err.message }};
    }}
  }},

  async registerDiscovery(): Promise<void> {{
    console.log('[{name}] registered with discovery layer (ADK)');
  }},
}};
"#
    )
}

// ── Template: TypeScript × crewai — fallback (Python-only) ───────────────────

fn template_ts_crewai(name: &str, caps: &[String]) -> String {
    template_ts_none(name, caps)
}

// ── Template: TypeScript × langgraph ─────────────────────────────────────────

fn template_ts_langgraph(name: &str, caps: &[String]) -> String {
    let name_lower = name.to_lowercase();
    let cap_list_sq: Vec<String> = caps.iter().map(|c| format!("'{c}'")).collect();
    let _cap_list = cap_list_sq.join(", ");
    let tags_with_lg = {
        let mut v = cap_list_sq.clone();
        v.push("'langgraph'".to_string());
        v.join(", ")
    };

    let tool_defs: String = caps
        .iter()
        .map(|c| {
            format!(
                r#"
const {c}Tool = tool(
  async ({{ query }}: {{ query: string }}) => {{
    // TODO: implement {c}
    return `{c} result for: ${{query}}`;
  }},
  {{
    name:        '{c}',
    description: '{c} — replace this with your real description.',
    schema:      z.object({{ query: z.string().describe('Input for {c}') }}),
  }},
);"#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let tool_refs: Vec<String> = caps.iter().map(|c| format!("{c}Tool")).collect();
    let tool_refs_str = tool_refs.join(", ");

    format!(
        r#"/**
 * {name} — LangGraph ReAct agent, wrapped for the Sentrix mesh.
 *
 * Install: npm install @langchain/langgraph @langchain/openai @langchain/core zod
 */
import {{ tool }}              from '@langchain/core/tools';
import {{ ChatOpenAI }}        from '@langchain/openai';
import {{ createReactAgent }}  from '@langchain/langgraph/prebuilt';
import {{ wrapLangGraph }}     from '../plugins/LangGraphPlugin';
import {{ z }}                 from 'zod';


// ── Define one tool per capability ────────────────────────────────────────────
{tool_defs}


// ── Build the LangGraph ReAct agent ──────────────────────────────────────────
const _llm   = new ChatOpenAI({{ model: 'gpt-4o-mini' }});
const _graph = createReactAgent({{ llm: _llm, tools: [{tool_refs_str}] }});


// ── Wrap for Sentrix ──────────────────────────────────────────────────────────
// After this call, {name} implements IAgent and is fully discoverable.
export const {name} = wrapLangGraph(_graph, {{
  name:           '{name}',
  agentId:        'sentrix://agent/{name_lower}',
  owner:          '0xYourWalletAddress',
  tags:           [{tags_with_lg}],
  stateInputKey:  'messages',
  stateOutputKey: 'messages',
}});
"#
    )
}

// ── Template: TypeScript × agno — fallback (Python-only) ─────────────────────

fn template_ts_agno(name: &str, caps: &[String]) -> String {
    template_ts_none(name, caps)
}

// ── Template: TypeScript × llamaindex ────────────────────────────────────────

fn template_ts_llamaindex(name: &str, caps: &[String]) -> String {
    let name_lower = name.to_lowercase();
    let cap_list_sq: Vec<String> = caps.iter().map(|c| format!("'{c}'")).collect();
    let cap_list = cap_list_sq.join(", ");
    let tags_with_li = {
        let mut v = cap_list_sq.clone();
        v.push("'llamaindex'".to_string());
        v.join(", ")
    };

    let fn_defs: String = caps.iter().map(|c| {
        format!(
            "\nfunction {c}({{ query }}: {{ query: string }}): string {{\n  // TODO: implement {c}\n  return `{c} result for: ${{query}}`;\n}}"
        )
    }).collect::<Vec<_>>().join("\n");

    let tool_vars: String = caps.iter().map(|c| {
        let cap_title = {
            let mut chars = c.chars();
            match chars.next() {
                None    => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
            }
        };
        format!(
            "const _tool{cap_title} = FunctionTool.from(\n  {c},\n  {{ name: '{c}', description: '{c} — replace with your real description.' }}\n);"
        )
    }).collect::<Vec<_>>().join("\n");

    let tool_refs: Vec<String> = caps
        .iter()
        .map(|c| {
            let cap_title = {
                let mut chars = c.chars();
                match chars.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
                }
            };
            format!("_tool{cap_title}")
        })
        .collect();
    let tool_refs_str = tool_refs.join(", ");

    format!(
        r#"/**
 * {name} — LlamaIndex agent, wrapped for the Sentrix mesh.
 *
 * Install: npm install llamaindex @llamaindex/openai
 */
import {{ OpenAI, OpenAIAgent, FunctionTool }} from 'llamaindex';
import {{ IAgent }}        from '../interfaces/IAgent';
import {{ AgentRequest }}  from '../interfaces/IAgentRequest';
import {{ AgentResponse }} from '../interfaces/IAgentResponse';


// ── Define one function per capability ────────────────────────────────────────
{fn_defs}


// ── Build the LlamaIndex agent ────────────────────────────────────────────────
{tool_vars}

const _llm   = new OpenAI({{ model: 'gpt-4o-mini' }});
const _agent = new OpenAIAgent({{
  tools: [{tool_refs_str}],
  llm:   _llm,
}});


// ── Sentrix-compliant wrapper ─────────────────────────────────────────────────
export const {name}: IAgent = {{
  agentId:  'sentrix://agent/{name_lower}',
  owner:    '0xYourWalletAddress',
  metadata: {{ name: '{name}', version: '0.1.0', tags: [{tags_with_li}] }},

  getCapabilities: () => [{cap_list}],

  async handleRequest(req: AgentRequest): Promise<AgentResponse> {{
    try {{
      const result = await _agent.chat({{ message: JSON.stringify(req.payload) }});
      return {{ requestId: req.requestId, status: 'success', result: {{ content: result.message.content }} }};
    }} catch (err: any) {{
      return {{ requestId: req.requestId, status: 'error', errorMessage: err.message }};
    }}
  }},

  async registerDiscovery(): Promise<void> {{
    console.log('[{name}] registered with discovery layer (LlamaIndex)');
  }},
}};
"#
    )
}

// ── Template: TypeScript × smolagents — fallback (Python-only) ───────────────

fn template_ts_smolagents(name: &str, caps: &[String]) -> String {
    template_ts_none(name, caps)
}

// ── Template: Python × none ───────────────────────────────────────────────────

fn template_py_none(name: &str, caps: &[String]) -> String {
    let snake = snake_case(name);
    let cap_list_sq: Vec<String> = caps.iter().map(|c| format!("'{c}'")).collect();
    let cap_list = cap_list_sq.join(", ");

    let handlers: String = caps.iter().map(|c| {
        format!("            '{c}': lambda r: AgentResponse.success(r.request_id, {{'message': '{c} called'}})")
    }).collect::<Vec<_>>().join(",\n");

    format!(
        r#"from interfaces.iagent         import IAgent
from interfaces.agent_request  import AgentRequest
from interfaces.agent_response import AgentResponse
from typing import List

class {name}(IAgent):
    agent_id = 'sentrix://agent/{snake}'
    owner    = '0xYourWalletAddress'
    metadata = {{'name': '{name}', 'version': '0.1.0', 'tags': [{cap_list}]}}

    def get_capabilities(self) -> List[str]:
        return [{cap_list}]

    async def handle_request(self, req: AgentRequest) -> AgentResponse:
        handlers = {{
{handlers}
        }}
        handler = handlers.get(req.capability)
        if handler:
            return handler(req)
        return AgentResponse.error(req.request_id, f'Unknown capability: {{req.capability}}')

    async def register_discovery(self) -> None:
        print(f'[{name}] registered with discovery layer')
"#
    )
}

// ── Template: Python × google-adk ────────────────────────────────────────────

fn template_py_google_adk(name: &str, caps: &[String]) -> String {
    let snake = snake_case(name);

    let fn_defs: String = caps
        .iter()
        .map(|c| {
            format!(
                r#"
def {c}(query: str) -> str:
    """{c} — replace this with your real implementation."""
    # TODO: implement {c}
    return f"{c} result for: {{query}}""#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let function_tools: Vec<String> = caps.iter().map(|c| format!("FunctionTool({c})")).collect();
    let function_tools_str = function_tools.join(", ");

    let tags_dq: Vec<String> = caps.iter().map(|c| format!(r#""{c}""#)).collect();
    let tags_str = {
        let mut v = tags_dq.clone();
        v.push(r#""adk""#.to_string());
        v.join(", ")
    };

    format!(
        r#""""
{name} — Google ADK agent, wrapped for the Sentrix mesh.

Each function below becomes one Sentrix capability.  The wrap_google_adk()
call translates between Sentrix AgentRequest / AgentResponse and the ADK
Runner transparently.
"""
from google.adk.agents          import Agent
from google.adk.tools           import FunctionTool
from plugins.google_adk_plugin  import wrap_google_adk


# ── Define one function per capability ────────────────────────────────────────
{fn_defs}


# ── Build the Google ADK agent ────────────────────────────────────────────────
_adk_agent = Agent(
    name        = "{snake}",
    model       = "gemini-2.0-flash",
    description = "{name} — built with Google ADK",
    tools       = [{function_tools_str}],
)


# ── Wrap for Sentrix ──────────────────────────────────────────────────────────
# After this call, {name} implements IAgent and is fully discoverable.
{name} = wrap_google_adk(
    agent    = _adk_agent,
    name     = "{name}",
    agent_id = "sentrix://agent/{snake}",
    owner    = "0xYourWalletAddress",
    tags     = [{tags_str}],
)
"#
    )
}

// ── Template: Python × crewai ─────────────────────────────────────────────────

fn template_py_crewai(name: &str, caps: &[String]) -> String {
    let snake = snake_case(name);

    let tool_defs: String = caps
        .iter()
        .map(|c| {
            format!(
                r#"
@tool("{c}")
def {c}(query: str) -> str:
    """{c} — replace this with your real implementation."""
    # TODO: implement {c}
    return f"{c} result for: {{query}}""#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let caps_joined = caps
        .iter()
        .map(|c| c.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let tools_list = caps_joined.clone();

    let tags_dq: Vec<String> = caps.iter().map(|c| format!(r#""{c}""#)).collect();
    let tags_str = {
        let mut v = tags_dq.clone();
        v.push(r#""crewai""#.to_string());
        v.join(", ")
    };

    format!(
        r#""""
{name} — CrewAI agent, wrapped for the Sentrix mesh.

Each @tool-decorated function becomes one Sentrix capability.  The
wrap_crewai() call handles AgentRequest / AgentResponse translation and
runs tasks via a single-agent Crew.
"""
from crewai              import Agent as CrewAgent
from crewai.tools        import tool
from plugins.crewai_plugin import wrap_crewai


# ── Define one @tool per capability ───────────────────────────────────────────
{tool_defs}


# ── Build the CrewAI agent ────────────────────────────────────────────────────
_crew_agent = CrewAgent(
    role      = "{name} Agent",
    goal      = "Perform {caps_joined} tasks accurately and helpfully.",
    backstory = (
        "You are {name}, a specialised AI agent. "
        "You excel at {caps_joined} and always produce high-quality results."
    ),
    tools     = [{tools_list}],
    verbose   = False,
)


# ── Wrap for Sentrix ──────────────────────────────────────────────────────────
# After this call, {name} implements IAgent and is fully discoverable.
{name} = wrap_crewai(
    agent    = _crew_agent,
    name     = "{name}",
    agent_id = "sentrix://agent/{snake}",
    owner    = "0xYourWalletAddress",
    tags     = [{tags_str}],
)
"#
    )
}

// ── Template: Python × langgraph ─────────────────────────────────────────────

fn template_py_langgraph(name: &str, caps: &[String]) -> String {
    let snake = snake_case(name);

    let tool_defs: String = caps
        .iter()
        .map(|c| {
            format!(
                r#"
@tool
def {c}(query: str) -> str:
    """{c} — replace this with your real implementation."""
    # TODO: implement {c}
    return f"{c} result for: {{query}}""#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let caps_joined = caps
        .iter()
        .map(|c| c.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let tools_list = caps_joined.clone();

    let tags_dq: Vec<String> = caps.iter().map(|c| format!(r#""{c}""#)).collect();
    let tags_str = {
        let mut v = tags_dq.clone();
        v.push(r#""langgraph""#.to_string());
        v.join(", ")
    };

    format!(
        r#""""
{name} — LangGraph ReAct agent, wrapped for the Sentrix mesh.

Each @tool-decorated function becomes one Sentrix capability.  The
wrap_langgraph() call handles AgentRequest / AgentResponse translation
and drives the graph via standard LangGraph invocation.
"""
from langchain_core.tools       import tool
from langchain_openai            import ChatOpenAI
from langgraph.prebuilt          import create_react_agent
from plugins.langgraph_plugin    import wrap_langgraph


# ── Define one @tool per capability ───────────────────────────────────────────
{tool_defs}


# ── Build the LangGraph ReAct agent ──────────────────────────────────────────
_llm   = ChatOpenAI(model="gpt-4o-mini")
_graph = create_react_agent(_llm, tools=[{tools_list}])


# ── Wrap for Sentrix ──────────────────────────────────────────────────────────
# After this call, {name} implements IAgent and is fully discoverable.
{name} = wrap_langgraph(
    graph    = _graph,
    name     = "{name}",
    agent_id = "sentrix://agent/{snake}",
    owner    = "0xYourWalletAddress",
    tags     = [{tags_str}],
    tools    = [{tools_list}],   # explicit list speeds up capability discovery
)
"#
    )
}

// ── Template: Python × agno ───────────────────────────────────────────────────

fn template_py_agno(name: &str, caps: &[String]) -> String {
    let snake = snake_case(name);

    let fn_defs: String = caps
        .iter()
        .map(|c| {
            format!(
                r#"
def {c}(query: str) -> str:
    """{c} — replace this with your real implementation."""
    # TODO: implement {c}
    return f"{c} result for: {{query}}""#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let tools_list = caps
        .iter()
        .map(|c| c.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    let tags_dq: Vec<String> = caps.iter().map(|c| format!(r#""{c}""#)).collect();
    let tags_str = {
        let mut v = tags_dq.clone();
        v.push(r#""agno""#.to_string());
        v.join(", ")
    };

    format!(
        r#""""
{name} — Agno agent, wrapped for the Sentrix mesh.

Each tool function becomes one Sentrix capability.  wrap_agno() handles
AgentRequest / AgentResponse translation automatically.
"""
from agno.agent              import Agent
from agno.models.openai      import OpenAIChat
from plugins.agno_plugin     import wrap_agno


# ── Define one function per capability ────────────────────────────────────────
{fn_defs}


# ── Build the Agno agent ──────────────────────────────────────────────────────
_agno_agent = Agent(
    model       = OpenAIChat(id="gpt-4o-mini"),
    tools       = [{tools_list}],
    description = "{name} — built with Agno",
    show_tool_calls = False,
)


# ── Wrap for Sentrix ──────────────────────────────────────────────────────────
{name} = wrap_agno(
    agent    = _agno_agent,
    name     = "{name}",
    agent_id = "sentrix://agent/{snake}",
    owner    = "0xYourWalletAddress",
    tags     = [{tags_str}],
)
"#
    )
}

// ── Template: Python × llamaindex ─────────────────────────────────────────────

fn template_py_llamaindex(name: &str, caps: &[String]) -> String {
    let snake = snake_case(name);

    let fn_defs: String = caps
        .iter()
        .map(|c| {
            format!(
                r#"
def {c}(query: str) -> str:
    """{c} — replace this with your real implementation."""
    # TODO: implement {c}
    return f"{c} result for: {{query}}""#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let tool_vars: String = caps
        .iter()
        .map(|c| format!("_tool_{c} = FunctionTool.from_defaults(fn={c})"))
        .collect::<Vec<_>>()
        .join("\n");

    let tool_refs: Vec<String> = caps.iter().map(|c| format!("_tool_{c}")).collect();
    let tool_refs_str = tool_refs.join(", ");

    let tags_dq: Vec<String> = caps.iter().map(|c| format!(r#""{c}""#)).collect();
    let tags_str = {
        let mut v = tags_dq.clone();
        v.push(r#""llamaindex""#.to_string());
        v.join(", ")
    };

    format!(
        r#""""
{name} — LlamaIndex ReAct agent, wrapped for the Sentrix mesh.

Each FunctionTool becomes one Sentrix capability.  wrap_llamaindex() handles
AgentRequest / AgentResponse translation automatically.
"""
from llama_index.core.agent         import ReActAgent
from llama_index.core.tools         import FunctionTool
from llama_index.llms.openai        import OpenAI
from plugins.llamaindex_plugin      import wrap_llamaindex


# ── Define one function per capability ────────────────────────────────────────
{fn_defs}


# ── Build the LlamaIndex agent ────────────────────────────────────────────────
{tool_vars}

_llm   = OpenAI(model="gpt-4o-mini")
_agent = ReActAgent.from_tools(
    [{tool_refs_str}],
    llm     = _llm,
    verbose = False,
)


# ── Wrap for Sentrix ──────────────────────────────────────────────────────────
{name} = wrap_llamaindex(
    agent    = _agent,
    name     = "{name}",
    agent_id = "sentrix://agent/{snake}",
    owner    = "0xYourWalletAddress",
    tags     = [{tags_str}],
    tools    = [{tool_refs_str}],
)
"#
    )
}

// ── Template: Python × smolagents ────────────────────────────────────────────

fn template_py_smolagents(name: &str, caps: &[String]) -> String {
    let snake = snake_case(name);

    let tool_defs: String = caps
        .iter()
        .map(|c| {
            format!(
                r#"
@tool
def {c}(query: str) -> str:
    """{c} — replace this with your real implementation.

    Args:
        query: The input for {c}.

    Returns:
        The result of {c}.
    """
    # TODO: implement {c}
    return f"{c} result for: {{query}}""#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let tools_list = caps
        .iter()
        .map(|c| c.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    let tags_dq: Vec<String> = caps.iter().map(|c| format!(r#""{c}""#)).collect();
    let tags_str = {
        let mut v = tags_dq.clone();
        v.push(r#""smolagents""#.to_string());
        v.join(", ")
    };

    format!(
        r#""""
{name} — smolagents agent, wrapped for the Sentrix mesh.

Each @tool-decorated function becomes one Sentrix capability.
wrap_smolagents() handles AgentRequest / AgentResponse translation.

Docs: https://huggingface.co/docs/smolagents
"""
from smolagents                   import ToolCallingAgent, tool
from smolagents.models            import HfApiModel
from plugins.smolagents_plugin    import wrap_smolagents


# ── Define one @tool per capability ───────────────────────────────────────────
# smolagents requires detailed Args/Returns docstrings for tool schema generation.
{tool_defs}


# ── Build the smolagents agent ────────────────────────────────────────────────
_agent = ToolCallingAgent(
    tools = [{tools_list}],
    model = HfApiModel("Qwen/Qwen2.5-72B-Instruct"),
    # For OpenAI: from smolagents.models import OpenAIServerModel
    # model = OpenAIServerModel(model_id="gpt-4o-mini")
)


# ── Wrap for Sentrix ──────────────────────────────────────────────────────────
{name} = wrap_smolagents(
    agent    = _agent,
    name     = "{name}",
    agent_id = "sentrix://agent/{snake}",
    owner    = "0xYourWalletAddress",
    tags     = [{tags_str}],
)
"#
    )
}

// ── Template: Rust × none ─────────────────────────────────────────────────────

fn template_rs_none(name: &str, caps: &[String]) -> String {
    let name_lower = name.to_lowercase();
    let caps_vec = caps_as_rs(caps);

    let match_arms: String = caps.iter().map(|c| {
        format!(r#"            "{c}" => AgentResponse::success(req.request_id, json!({{ "message": "{c} called" }}))"#)
    }).collect::<Vec<_>>().join(",\n");

    format!(
        r#"use crate::agent::IAgent;
use crate::request::AgentRequest;
use crate::response::AgentResponse;
use async_trait::async_trait;
use serde_json::json;

pub struct {name};

#[async_trait]
impl IAgent for {name} {{
    fn agent_id(&self) -> &str {{ "sentrix://agent/{name_lower}" }}
    fn owner(&self)    -> &str {{ "0xYourWalletAddress" }}

    fn get_capabilities(&self) -> Vec<String> {{
        {caps_vec}
    }}

    async fn handle_request(&self, req: AgentRequest) -> AgentResponse {{
        match req.capability.as_str() {{
{match_arms},
            _ => AgentResponse::error(req.request_id, format!("Unknown: {{}}", req.capability)),
        }}
    }}
}}
"#
    )
}

// ── Template: Rust × google-adk — stub (no Rust SDK) ─────────────────────────

fn template_rs_google_adk(name: &str, caps: &[String]) -> String {
    template_rs_none(name, caps)
}

fn template_rs_crewai(name: &str, caps: &[String]) -> String {
    template_rs_none(name, caps)
}

fn template_rs_langgraph(name: &str, caps: &[String]) -> String {
    template_rs_none(name, caps)
}

fn template_rs_agno(name: &str, caps: &[String]) -> String {
    template_rs_none(name, caps)
}

fn template_rs_llamaindex(name: &str, caps: &[String]) -> String {
    template_rs_none(name, caps)
}

fn template_rs_smolagents(name: &str, caps: &[String]) -> String {
    template_rs_none(name, caps)
}

// ── Template: Zig × none ──────────────────────────────────────────────────────

fn template_zig_none(name: &str, caps: &[String]) -> String {
    let name_lower = name.to_lowercase();
    let caps_zig = caps_as_zig(caps);

    // Build if/else if chain
    let if_chain: String = caps.iter().enumerate().map(|(i, c)| {
        let kw = if i == 0 { "if" } else { "} else if" };
        format!(
            r#"        {kw} (std.mem.eql(u8, req.capability, "{c}")) {{
            return .{{ .request_id = req.request_id, .status = "success", .result = "{c} called" }};"#
        )
    }).collect::<Vec<_>>().join("\n");

    format!(
        r#"const std   = @import("std");
const types = @import("../interfaces/types.zig");

pub const {name} = struct {{
    agent_id: []const u8 = "sentrix://agent/{name_lower}",
    owner:    []const u8 = "0xYourWalletAddress",

    pub fn getCapabilities(_: *const {name}) []const []const u8 {{
        return {caps_zig};
    }}

    pub fn handleRequest(_: *const {name}, req: types.AgentRequest) types.AgentResponse {{
{if_chain}
        }} else {{
            return .{{ .request_id = req.request_id, .status = "error", .result = "Unknown capability" }};
        }}
    }}
}};
"#
    )
}

fn template_zig_google_adk(name: &str, caps: &[String]) -> String {
    template_zig_none(name, caps)
}

fn template_zig_crewai(name: &str, caps: &[String]) -> String {
    template_zig_none(name, caps)
}

fn template_zig_langgraph(name: &str, caps: &[String]) -> String {
    template_zig_none(name, caps)
}

fn template_zig_agno(name: &str, caps: &[String]) -> String {
    template_zig_none(name, caps)
}

fn template_zig_llamaindex(name: &str, caps: &[String]) -> String {
    template_zig_none(name, caps)
}

fn template_zig_smolagents(name: &str, caps: &[String]) -> String {
    template_zig_none(name, caps)
}

// ── Template dispatch ─────────────────────────────────────────────────────────

fn generate(lang: &Lang, framework: &Framework, name: &str, caps: &[String]) -> String {
    match (lang, framework) {
        // TypeScript
        (Lang::TypeScript, Framework::None) => template_ts_none(name, caps),
        (Lang::TypeScript, Framework::GoogleAdk) => template_ts_google_adk(name, caps),
        (Lang::TypeScript, Framework::CrewAi) => template_ts_crewai(name, caps),
        (Lang::TypeScript, Framework::LangGraph) => template_ts_langgraph(name, caps),
        (Lang::TypeScript, Framework::Agno) => template_ts_agno(name, caps),
        (Lang::TypeScript, Framework::LlamaIndex) => template_ts_llamaindex(name, caps),
        (Lang::TypeScript, Framework::Smolagents) => template_ts_smolagents(name, caps),
        // Python
        (Lang::Python, Framework::None) => template_py_none(name, caps),
        (Lang::Python, Framework::GoogleAdk) => template_py_google_adk(name, caps),
        (Lang::Python, Framework::CrewAi) => template_py_crewai(name, caps),
        (Lang::Python, Framework::LangGraph) => template_py_langgraph(name, caps),
        (Lang::Python, Framework::Agno) => template_py_agno(name, caps),
        (Lang::Python, Framework::LlamaIndex) => template_py_llamaindex(name, caps),
        (Lang::Python, Framework::Smolagents) => template_py_smolagents(name, caps),
        // Rust
        (Lang::Rust, Framework::None) => template_rs_none(name, caps),
        (Lang::Rust, Framework::GoogleAdk) => template_rs_google_adk(name, caps),
        (Lang::Rust, Framework::CrewAi) => template_rs_crewai(name, caps),
        (Lang::Rust, Framework::LangGraph) => template_rs_langgraph(name, caps),
        (Lang::Rust, Framework::Agno) => template_rs_agno(name, caps),
        (Lang::Rust, Framework::LlamaIndex) => template_rs_llamaindex(name, caps),
        (Lang::Rust, Framework::Smolagents) => template_rs_smolagents(name, caps),
        // Zig
        (Lang::Zig, Framework::None) => template_zig_none(name, caps),
        (Lang::Zig, Framework::GoogleAdk) => template_zig_google_adk(name, caps),
        (Lang::Zig, Framework::CrewAi) => template_zig_crewai(name, caps),
        (Lang::Zig, Framework::LangGraph) => template_zig_langgraph(name, caps),
        (Lang::Zig, Framework::Agno) => template_zig_agno(name, caps),
        (Lang::Zig, Framework::LlamaIndex) => template_zig_llamaindex(name, caps),
        (Lang::Zig, Framework::Smolagents) => template_zig_smolagents(name, caps),
    }
}

// ── Output filename ───────────────────────────────────────────────────────────

fn agent_filename(lang: &Lang, name: &str) -> String {
    match lang {
        Lang::TypeScript => format!("{}.ts", pascal_case(name)),
        Lang::Python => format!("{}.py", snake_case(name)),
        Lang::Rust => format!("{}.rs", snake_case(name)),
        Lang::Zig => format!("{}.zig", snake_case(name)),
    }
}

// ── run ───────────────────────────────────────────────────────────────────────

pub fn run(args: CreateArgs) -> Result<()> {
    // 1. Validate subcommand
    if args.subcommand != "agent" {
        eprintln!("Usage: sentrix create agent <name> [options]");
        eprintln!("       sentrix create agent --help");
        return Ok(());
    }

    // 2. Agent name
    let name = match &args.name {
        Some(n) => n.clone(),
        None => bail!("Agent name required: sentrix create agent <name>"),
    };

    // 3. Detect language
    let cwd = std::env::current_dir()?;
    let lang = detect_lang::detect(args.lang.as_deref(), &cwd)?;

    // 4. Parse framework
    let framework = Framework::from_str(&args.framework);

    // Warn about Python-only frameworks
    if framework.is_python_only() && lang != Lang::Python {
        logger::warn(&format!(
            "The \"{}\" framework does not have a {} template yet. Generating a plain IAgent instead.",
            framework.as_str(), lang
        ));
    }

    // 5. Parse capabilities
    let mut caps: Vec<String> = args
        .capabilities
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if caps.is_empty() {
        caps = vec!["echo".to_string(), "ping".to_string()];
    }

    // 6. Detect project root and agents/ directory
    let project_root = detect_lang::find_project_root(&cwd);
    let agents_dir = project_root.join("agents");

    if !agents_dir.exists() {
        logger::error("No \"agents/\" folder found. Are you inside a Sentrix project? Run sentrix init first.");
        return Ok(());
    }

    // 7. Confirmation
    let framework_display = if framework == Framework::None {
        lang.to_string()
    } else {
        format!("{}, {}", lang, framework.as_str())
    };

    if !args.yes {
        print!("Create agent '{}' ({})? [Y/n]: ", name, framework_display);
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();
        if trimmed == "n" || trimmed == "N" {
            logger::info("Aborted.");
            return Ok(());
        }
    }

    // 8. Generate file content
    let content = generate(&lang, &framework, &name, &caps);

    // 9. Write to agents/<filename>
    let filename = agent_filename(&lang, &name);
    let dest = agents_dir.join(&filename);

    if dest.exists() {
        logger::error(&format!("Agent file \"{}\" already exists.", filename));
        return Ok(());
    }

    std::fs::write(&dest, &content)?;

    // 10. Success message
    logger::success(&format!("Agent created: agents/{}", filename));
    logger::info(&format!("Capabilities : {}", caps.join(", ")));
    if framework != Framework::None {
        logger::info(&format!("Framework    : {}", framework.as_str()));
    }

    // 11. Offer to install deps (unless --yes)
    if let Some(hint) = framework.install_hint(&lang) {
        println!();
        println!("  Framework dependencies:");
        println!("    {}", hint);
        println!();

        let should_install = if args.yes {
            true
        } else {
            print!("Install {} dependencies now? [Y/n]: ", framework.as_str());
            io::stdout().flush()?;
            let mut ans = String::new();
            io::stdin().read_line(&mut ans)?;
            let trimmed = ans.trim();
            !(trimmed == "n" || trimmed == "N")
        };

        if should_install {
            install_deps(&lang, &project_root);
        } else {
            logger::dim(&format!("  Skipped. Run later:  {}", hint));
        }
    }

    // 12. x402 add-on hint
    if args.addon.as_deref() == Some("x402") {
        let first_cap = caps.first().map(|s| s.as_str()).unwrap_or("myCapability");
        let x402_snippet = if lang == Lang::Python {
            format!(
                "\nfrom addons.x402 import X402ServerMixin, CapabilityPricing\n\
                 # Mixin: class {name}(X402ServerMixin, IAgent):\n\
                 #   x402_pricing = {{ '{first_cap}': CapabilityPricing.usdc_base(50, '0xMyWallet') }}"
            )
        } else {
            format!(
                "\nimport {{ withX402Payment, usdcBase }} from '../addons/x402';\n\
                 // const agent = withX402Payment(new {name}(), {{ pricing: {{ '{first_cap}': usdcBase(50, '0xMyWallet') }} }});"
            )
        };
        println!();
        println!("  x402 payment add-on enabled:");
        println!("{}", x402_snippet);
        println!("  Docs: docs/x402.md");
    }

    // Next steps
    println!();
    println!("  Next steps:");
    println!(
        "    1. Edit  agents/{}  — fill in your implementations",
        filename
    );
    println!(
        "    2. Register your agent: await {}.register_discovery()",
        name
    );
    println!(
        "    3. Query from another agent: registry.query('{}')",
        caps.first().map(|s| s.as_str()).unwrap_or("yourCapability")
    );
    if args.addon.as_deref() == Some("x402") {
        println!("    4. Add payment: docs/x402.md");
    }

    Ok(())
}
