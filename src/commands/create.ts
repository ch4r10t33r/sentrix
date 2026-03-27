import path       from 'path';
import fs         from 'fs-extra';
import ora        from 'ora';
import chalk      from 'chalk';
import inquirer   from 'inquirer';
import { execSync } from 'child_process';
import { detectLanguage } from '../utils/detect-lang';
import { logger }         from '../utils/logger';

// ── Types ─────────────────────────────────────────────────────────────────────

export type Framework = 'none' | 'google-adk' | 'crewai' | 'langgraph' | 'agno' | 'llamaindex' | 'smolagents';

interface CreateOptions {
  lang?:         string;
  capabilities:  string;
  framework:     Framework;
  addon?:        string;   // e.g. 'x402'
  yes?:          boolean;  // skip confirmation prompts (CI / --yes flag)
}

type TemplateFn = (name: string, caps: string[]) => string;
type LangTemplates = Partial<Record<string, TemplateFn>>;

// ── Helper: snake_case for Python filenames ───────────────────────────────────

function toSnake(name: string): string {
  return name.replace(/([A-Z])/g, '_$1').toLowerCase().replace(/^_/, '');
}

// ── Framework install hints ───────────────────────────────────────────────────

const INSTALL_HINTS: Record<Framework, Partial<Record<string, string>>> = {
  'none': {},
  'google-adk': {
    typescript: 'npm install @google/generative-ai @google-labs/agent-development-kit',
    python:     'pip install google-adk google-generativeai',
  },
  'crewai': {
    python: 'pip install crewai crewai-tools',
  },
  'langgraph': {
    typescript: 'npm install @langchain/langgraph @langchain/openai @langchain/core zod',
    python:     'pip install langgraph langchain-openai langchain-core',
  },
  'agno': {
    python: 'pip install agno openai',
  },
  'llamaindex': {
    typescript: 'npm install llamaindex @llamaindex/openai',
    python:     'pip install llama-index llama-index-llms-openai',
  },
  'smolagents': {
    python: 'pip install smolagents',
  },
};

// ── FRAMEWORK: none (plain IAgent) ────────────────────────────────────────────

const PLAIN_TEMPLATES: LangTemplates = {

  typescript: (name, caps) => `import { IAgent }        from '../interfaces/IAgent';
import { AgentRequest }  from '../interfaces/IAgentRequest';
import { AgentResponse } from '../interfaces/IAgentResponse';

export class ${name} implements IAgent {
  readonly agentId  = 'borgkit://agent/${name.toLowerCase()}';
  readonly owner    = '0xYourWalletAddress';
  readonly metadata = { name: '${name}', version: '0.1.0', tags: [${caps.map(c => `'${c}'`).join(', ')}] };

  getCapabilities(): string[] {
    return [${caps.map(c => `'${c}'`).join(', ')}];
  }

  async handleRequest(req: AgentRequest): Promise<AgentResponse> {
    switch (req.capability) {
${caps.map(c => `      case '${c}':\n        // TODO: implement ${c}\n        return { requestId: req.requestId, status: 'success', result: { message: '${c} called' } };`).join('\n')}
      default:
        return { requestId: req.requestId, status: 'error', errorMessage: \`Unknown capability: \${req.capability}\` };
    }
  }

  async registerDiscovery(): Promise<void> {
    // TODO: plug in your discovery adapter
    console.log('[${name}] registered with discovery layer');
  }
}
`,

  python: (name, caps) => `from interfaces.iagent         import IAgent
from interfaces.agent_request  import AgentRequest
from interfaces.agent_response import AgentResponse
from typing import List

class ${name}(IAgent):
    agent_id = 'borgkit://agent/${toSnake(name)}'
    owner    = '0xYourWalletAddress'
    metadata = {'name': '${name}', 'version': '0.1.0', 'tags': [${caps.map(c => `'${c}'`).join(', ')}]}

    def get_capabilities(self) -> List[str]:
        return [${caps.map(c => `'${c}'`).join(', ')}]

    async def handle_request(self, req: AgentRequest) -> AgentResponse:
        handlers = {
${caps.map(c => `            '${c}': lambda r: AgentResponse.success(r.request_id, {'message': '${c} called'})`).join(',\n')}
        }
        handler = handlers.get(req.capability)
        if handler:
            return handler(req)
        return AgentResponse.error(req.request_id, f'Unknown capability: {req.capability}')

    async def register_discovery(self) -> None:
        print(f'[${name}] registered with discovery layer')
`,

  rust: (name, caps) => `use crate::agent::IAgent;
use crate::request::AgentRequest;
use crate::response::AgentResponse;
use async_trait::async_trait;
use serde_json::json;

pub struct ${name};

#[async_trait]
impl IAgent for ${name} {
    fn agent_id(&self) -> &str { "borgkit://agent/${name.toLowerCase()}" }
    fn owner(&self)    -> &str { "0xYourWalletAddress" }

    fn get_capabilities(&self) -> Vec<String> {
        vec![${caps.map(c => `"${c}".to_string()`).join(', ')}]
    }

    async fn handle_request(&self, req: AgentRequest) -> AgentResponse {
        match req.capability.as_str() {
${caps.map(c => `            "${c}" => AgentResponse::success(req.request_id, json!({ "message": "${c} called" }))`).join(',\n')},
            _ => AgentResponse::error(req.request_id, format!("Unknown: {}", req.capability)),
        }
    }
}
`,

  zig: (name, caps) => `const std   = @import("std");
const types = @import("../interfaces/types.zig");

pub const ${name} = struct {
    agent_id: []const u8 = "borgkit://agent/${name.toLowerCase()}",
    owner:    []const u8 = "0xYourWalletAddress",

    pub fn getCapabilities(_: *const ${name}) []const []const u8 {
        return &.{ ${caps.map(c => `"${c}"`).join(', ')} };
    }

    pub fn handleRequest(_: *const ${name}, req: types.AgentRequest) types.AgentResponse {
        ${caps.map((c, i) => `${i === 0 ? 'if' : '} else if'} (std.mem.eql(u8, req.capability, "${c}")) {
            return .{ .request_id = req.request_id, .status = "success", .result = "${c} called" };`).join('\n        ')}
        } else {
            return .{ .request_id = req.request_id, .status = "error", .result = "Unknown capability" };
        }
    }
};
`,
};

// ── FRAMEWORK: google-adk ─────────────────────────────────────────────────────

const GOOGLE_ADK_TEMPLATES: LangTemplates = {

  python: (name, caps) => `"""
${name} — Google ADK agent, wrapped for the Borgkit mesh.

Each function below becomes one Borgkit capability.  The wrap_google_adk()
call translates between Borgkit AgentRequest / AgentResponse and the ADK
Runner transparently.
"""
from google.adk.agents          import Agent
from google.adk.tools           import FunctionTool
from plugins.google_adk_plugin  import wrap_google_adk


# ── Define one function per capability ────────────────────────────────────────
${caps.map(c => `
def ${c}(query: str) -> str:
    """${c} — replace this with your real implementation."""
    # TODO: implement ${c}
    return f"${c} result for: {query}"`).join('\n')}


# ── Build the Google ADK agent ────────────────────────────────────────────────
_adk_agent = Agent(
    name        = "${toSnake(name)}",
    model       = "gemini-2.0-flash",
    description = "${name} — built with Google ADK",
    tools       = [${caps.map(c => `FunctionTool(${c})`).join(', ')}],
)


# ── Wrap for Borgkit ──────────────────────────────────────────────────────────
# After this call, ${name} implements IAgent and is fully discoverable.
${name} = wrap_google_adk(
    agent    = _adk_agent,
    name     = "${name}",
    agent_id = "borgkit://agent/${toSnake(name)}",
    owner    = "0xYourWalletAddress",
    tags     = [${caps.map(c => `"${c}"`).join(', ')}, "adk"],
)
`,

  typescript: (name, caps) => `/**
 * ${name} — Google ADK agent, wrapped for the Borgkit mesh.
 *
 * Install: npm install @google-labs/agent-development-kit @google/generative-ai
 */
import { Agent, FunctionTool }  from '@google-labs/agent-development-kit';
import { IAgent }               from '../interfaces/IAgent';
import { AgentRequest }         from '../interfaces/IAgentRequest';
import { AgentResponse }        from '../interfaces/IAgentResponse';

// ── Define one function per capability ────────────────────────────────────────
${caps.map(c => `
function ${c}({ query }: { query: string }): string {
  // TODO: implement ${c}
  return \`${c} result for: \${query}\`;
}`).join('\n')}

// ── Build the Google ADK agent ────────────────────────────────────────────────
const _adkAgent = new Agent({
  name:        '${name.toLowerCase()}',
  model:       'gemini-2.0-flash',
  description: '${name} — built with Google ADK',
  tools:       [${caps.map(c => `new FunctionTool(${c})`).join(', ')}],
});

// ── Borgkit-compliant wrapper ─────────────────────────────────────────────────
// Adapts the ADK agent to the IAgent interface for full mesh interoperability.
export const ${name}: IAgent = {
  agentId:  'borgkit://agent/${name.toLowerCase()}',
  owner:    '0xYourWalletAddress',
  metadata: { name: '${name}', version: '0.1.0', tags: [${caps.map(c => `'${c}'`).join(', ')}, 'adk'] },

  getCapabilities: () => [${caps.map(c => `'${c}'`).join(', ')}],

  async handleRequest(req: AgentRequest): Promise<AgentResponse> {
    try {
      // Route to the ADK agent via its native runner
      const runner = _adkAgent.createRunner();
      const result = await runner.run({ input: JSON.stringify(req.payload) });
      return { requestId: req.requestId, status: 'success', result: { content: String(result) } };
    } catch (err: any) {
      return { requestId: req.requestId, status: 'error', errorMessage: err.message };
    }
  },

  async registerDiscovery(): Promise<void> {
    console.log('[${name}] registered with discovery layer (ADK)');
  },
};
`,
};

// ── FRAMEWORK: crewai ─────────────────────────────────────────────────────────

const CREWAI_TEMPLATES: LangTemplates = {

  python: (name, caps) => `"""
${name} — CrewAI agent, wrapped for the Borgkit mesh.

Each @tool-decorated function becomes one Borgkit capability.  The
wrap_crewai() call handles AgentRequest / AgentResponse translation and
runs tasks via a single-agent Crew.
"""
from crewai              import Agent as CrewAgent
from crewai.tools        import tool
from plugins.crewai_plugin import wrap_crewai


# ── Define one @tool per capability ───────────────────────────────────────────
${caps.map(c => `
@tool("${c}")
def ${c}(query: str) -> str:
    """${c} — replace this with your real implementation."""
    # TODO: implement ${c}
    return f"${c} result for: {query}"`).join('\n')}


# ── Build the CrewAI agent ────────────────────────────────────────────────────
_crew_agent = CrewAgent(
    role      = "${name} Agent",
    goal      = "Perform ${caps.join(', ')} tasks accurately and helpfully.",
    backstory = (
        "You are ${name}, a specialised AI agent. "
        "You excel at ${caps.join(', ')} and always produce high-quality results."
    ),
    tools     = [${caps.join(', ')}],
    verbose   = False,
)


# ── Wrap for Borgkit ──────────────────────────────────────────────────────────
# After this call, ${name} implements IAgent and is fully discoverable.
${name} = wrap_crewai(
    agent    = _crew_agent,
    name     = "${name}",
    agent_id = "borgkit://agent/${toSnake(name)}",
    owner    = "0xYourWalletAddress",
    tags     = [${caps.map(c => `"${c}"`).join(', ')}, "crewai"],
)
`,
};

// ── FRAMEWORK: langgraph ──────────────────────────────────────────────────────

const LANGGRAPH_TEMPLATES: LangTemplates = {

  python: (name, caps) => `"""
${name} — LangGraph ReAct agent, wrapped for the Borgkit mesh.

Each @tool-decorated function becomes one Borgkit capability.  The
wrap_langgraph() call handles AgentRequest / AgentResponse translation
and drives the graph via standard LangGraph invocation.
"""
from langchain_core.tools       import tool
from langchain_openai            import ChatOpenAI
from langgraph.prebuilt          import create_react_agent
from plugins.langgraph_plugin    import wrap_langgraph


# ── Define one @tool per capability ───────────────────────────────────────────
${caps.map(c => `
@tool
def ${c}(query: str) -> str:
    """${c} — replace this with your real implementation."""
    # TODO: implement ${c}
    return f"${c} result for: {query}"`).join('\n')}


# ── Build the LangGraph ReAct agent ──────────────────────────────────────────
_llm   = ChatOpenAI(model="gpt-4o-mini")
_graph = create_react_agent(_llm, tools=[${caps.join(', ')}])


# ── Wrap for Borgkit ──────────────────────────────────────────────────────────
# After this call, ${name} implements IAgent and is fully discoverable.
${name} = wrap_langgraph(
    graph    = _graph,
    name     = "${name}",
    agent_id = "borgkit://agent/${toSnake(name)}",
    owner    = "0xYourWalletAddress",
    tags     = [${caps.map(c => `"${c}"`).join(', ')}, "langgraph"],
    tools    = [${caps.join(', ')}],   # explicit list speeds up capability discovery
)
`,

  typescript: (name, caps) => `/**
 * ${name} — LangGraph ReAct agent, wrapped for the Borgkit mesh.
 *
 * Install: npm install @langchain/langgraph @langchain/openai @langchain/core zod
 */
import { tool }              from '@langchain/core/tools';
import { ChatOpenAI }        from '@langchain/openai';
import { createReactAgent }  from '@langchain/langgraph/prebuilt';
import { wrapLangGraph }     from '../plugins/LangGraphPlugin';
import { z }                 from 'zod';


// ── Define one tool per capability ────────────────────────────────────────────
${caps.map(c => `
const ${c}Tool = tool(
  async ({ query }: { query: string }) => {
    // TODO: implement ${c}
    return \`${c} result for: \${query}\`;
  },
  {
    name:        '${c}',
    description: '${c} — replace this with your real description.',
    schema:      z.object({ query: z.string().describe('Input for ${c}') }),
  },
);`).join('\n')}


// ── Build the LangGraph ReAct agent ──────────────────────────────────────────
const _llm   = new ChatOpenAI({ model: 'gpt-4o-mini' });
const _graph = createReactAgent({ llm: _llm, tools: [${caps.map(c => `${c}Tool`).join(', ')}] });


// ── Wrap for Borgkit ──────────────────────────────────────────────────────────
// After this call, ${name} implements IAgent and is fully discoverable.
export const ${name} = wrapLangGraph(_graph, {
  name:           '${name}',
  agentId:        'borgkit://agent/${name.toLowerCase()}',
  owner:          '0xYourWalletAddress',
  tags:           [${caps.map(c => `'${c}'`).join(', ')}, 'langgraph'],
  stateInputKey:  'messages',
  stateOutputKey: 'messages',
});
`,
};

// ── FRAMEWORK: agno ───────────────────────────────────────────────────────────

const AGNO_TEMPLATES: LangTemplates = {

  python: (name, caps) => `"""
${name} — Agno agent, wrapped for the Borgkit mesh.

Each tool function becomes one Borgkit capability.  wrap_agno() handles
AgentRequest / AgentResponse translation automatically.
"""
from agno.agent              import Agent
from agno.models.openai      import OpenAIChat
from plugins.agno_plugin     import wrap_agno


# ── Define one function per capability ────────────────────────────────────────
${caps.map(c => `
def ${c}(query: str) -> str:
    """${c} — replace this with your real implementation."""
    # TODO: implement ${c}
    return f"${c} result for: {query}"`).join('\n')}


# ── Build the Agno agent ──────────────────────────────────────────────────────
_agno_agent = Agent(
    model       = OpenAIChat(id="gpt-4o-mini"),
    tools       = [${caps.join(', ')}],
    description = "${name} — built with Agno",
    show_tool_calls = False,
)


# ── Wrap for Borgkit ──────────────────────────────────────────────────────────
${name} = wrap_agno(
    agent    = _agno_agent,
    name     = "${name}",
    agent_id = "borgkit://agent/${toSnake(name)}",
    owner    = "0xYourWalletAddress",
    tags     = [${caps.map(c => `"${c}"`).join(', ')}, "agno"],
)
`,
};

// ── FRAMEWORK: llamaindex ─────────────────────────────────────────────────────

const LLAMAINDEX_TEMPLATES: LangTemplates = {

  python: (name, caps) => `"""
${name} — LlamaIndex ReAct agent, wrapped for the Borgkit mesh.

Each FunctionTool becomes one Borgkit capability.  wrap_llamaindex() handles
AgentRequest / AgentResponse translation automatically.
"""
from llama_index.core.agent         import ReActAgent
from llama_index.core.tools         import FunctionTool
from llama_index.llms.openai        import OpenAI
from plugins.llamaindex_plugin      import wrap_llamaindex


# ── Define one function per capability ────────────────────────────────────────
${caps.map(c => `
def ${c}(query: str) -> str:
    """${c} — replace this with your real implementation."""
    # TODO: implement ${c}
    return f"${c} result for: {query}"`).join('\n')}


# ── Build the LlamaIndex agent ────────────────────────────────────────────────
${caps.map(c => `_tool_${c} = FunctionTool.from_defaults(fn=${c})`).join('\n')}

_llm   = OpenAI(model="gpt-4o-mini")
_agent = ReActAgent.from_tools(
    [${caps.map(c => `_tool_${c}`).join(', ')}],
    llm     = _llm,
    verbose = False,
)


# ── Wrap for Borgkit ──────────────────────────────────────────────────────────
${name} = wrap_llamaindex(
    agent    = _agent,
    name     = "${name}",
    agent_id = "borgkit://agent/${toSnake(name)}",
    owner    = "0xYourWalletAddress",
    tags     = [${caps.map(c => `"${c}"`).join(', ')}, "llamaindex"],
    tools    = [${caps.map(c => `_tool_${c}`).join(', ')}],
)
`,

  typescript: (name, caps) => `/**
 * ${name} — LlamaIndex agent, wrapped for the Borgkit mesh.
 *
 * Install: npm install llamaindex @llamaindex/openai
 */
import { OpenAI, OpenAIAgent, FunctionTool } from 'llamaindex';
import { IAgent }        from '../interfaces/IAgent';
import { AgentRequest }  from '../interfaces/IAgentRequest';
import { AgentResponse } from '../interfaces/IAgentResponse';


// ── Define one function per capability ────────────────────────────────────────
${caps.map(c => `
function ${c}({ query }: { query: string }): string {
  // TODO: implement ${c}
  return \`${c} result for: \${query}\`;
}`).join('\n')}


// ── Build the LlamaIndex agent ────────────────────────────────────────────────
${caps.map(c => `const _tool${c.charAt(0).toUpperCase() + c.slice(1)} = FunctionTool.from(
  ${c},
  { name: '${c}', description: '${c} — replace with your real description.' }
);`).join('\n')}

const _llm   = new OpenAI({ model: 'gpt-4o-mini' });
const _agent = new OpenAIAgent({
  tools: [${caps.map(c => `_tool${c.charAt(0).toUpperCase() + c.slice(1)}`).join(', ')}],
  llm:   _llm,
});


// ── Borgkit-compliant wrapper ─────────────────────────────────────────────────
export const ${name}: IAgent = {
  agentId:  'borgkit://agent/${name.toLowerCase()}',
  owner:    '0xYourWalletAddress',
  metadata: { name: '${name}', version: '0.1.0', tags: [${caps.map(c => `'${c}'`).join(', ')}, 'llamaindex'] },

  getCapabilities: () => [${caps.map(c => `'${c}'`).join(', ')}],

  async handleRequest(req: AgentRequest): Promise<AgentResponse> {
    try {
      const result = await _agent.chat({ message: JSON.stringify(req.payload) });
      return { requestId: req.requestId, status: 'success', result: { content: result.message.content } };
    } catch (err: any) {
      return { requestId: req.requestId, status: 'error', errorMessage: err.message };
    }
  },

  async registerDiscovery(): Promise<void> {
    console.log('[${name}] registered with discovery layer (LlamaIndex)');
  },
};
`,
};

// ── FRAMEWORK: smolagents ─────────────────────────────────────────────────────

const SMOLAGENTS_TEMPLATES: LangTemplates = {

  python: (name, caps) => `"""
${name} — smolagents agent, wrapped for the Borgkit mesh.

Each @tool-decorated function becomes one Borgkit capability.
wrap_smolagents() handles AgentRequest / AgentResponse translation.

Docs: https://huggingface.co/docs/smolagents
"""
from smolagents                   import ToolCallingAgent, tool
from smolagents.models            import HfApiModel
from plugins.smolagents_plugin    import wrap_smolagents


# ── Define one @tool per capability ───────────────────────────────────────────
# smolagents requires detailed Args/Returns docstrings for tool schema generation.
${caps.map(c => `
@tool
def ${c}(query: str) -> str:
    """${c} — replace this with your real implementation.

    Args:
        query: The input for ${c}.

    Returns:
        The result of ${c}.
    """
    # TODO: implement ${c}
    return f"${c} result for: {query}"`).join('\n')}


# ── Build the smolagents agent ────────────────────────────────────────────────
_agent = ToolCallingAgent(
    tools = [${caps.join(', ')}],
    model = HfApiModel("Qwen/Qwen2.5-72B-Instruct"),
    # For OpenAI: from smolagents.models import OpenAIServerModel
    # model = OpenAIServerModel(model_id="gpt-4o-mini")
)


# ── Wrap for Borgkit ──────────────────────────────────────────────────────────
${name} = wrap_smolagents(
    agent    = _agent,
    name     = "${name}",
    agent_id = "borgkit://agent/${toSnake(name)}",
    owner    = "0xYourWalletAddress",
    tags     = [${caps.map(c => `"${c}"`).join(', ')}, "smolagents"],
)
`,
};

// ── Template dispatch table ───────────────────────────────────────────────────

const FRAMEWORK_TEMPLATES: Record<Framework, LangTemplates> = {
  'none':        PLAIN_TEMPLATES,
  'google-adk':  GOOGLE_ADK_TEMPLATES,
  'crewai':      CREWAI_TEMPLATES,
  'langgraph':   LANGGRAPH_TEMPLATES,
  'agno':        AGNO_TEMPLATES,
  'llamaindex':  LLAMAINDEX_TEMPLATES,
  'smolagents':  SMOLAGENTS_TEMPLATES,
};

// ── createCommand ─────────────────────────────────────────────────────────────

export async function createCommand(
  agentName: string,
  options:   CreateOptions,
): Promise<void> {
  const projectDir = process.cwd();
  const lang       = options.lang ?? detectLanguage(projectDir);
  const framework  = options.framework ?? 'none';
  const caps       = options.capabilities.split(',').map(c => c.trim()).filter(Boolean);

  // ── Validate project dir ───────────────────────────────────────────────────
  const agentsDir = path.join(projectDir, 'agents');
  if (!fs.existsSync(agentsDir)) {
    logger.error('No "agents/" folder found. Are you inside a Borgkit project? Run borgkit init first.');
    process.exit(1);
  }

  // ── Resolve template ───────────────────────────────────────────────────────
  const langTemplates = FRAMEWORK_TEMPLATES[framework] ?? PLAIN_TEMPLATES;
  const templateFn    = langTemplates[lang] ?? PLAIN_TEMPLATES[lang];

  if (!templateFn) {
    if (!PLAIN_TEMPLATES[lang]) {
      logger.error(`No agent template for language "${lang}".`);
      process.exit(1);
    }
    // Framework not supported in this language — fall back with a warning
    logger.warn(
      `The "${framework}" framework does not have a ${lang} template yet. ` +
      `Generating a plain IAgent instead.`
    );
  }

  // ── Resolve filename ───────────────────────────────────────────────────────
  const extensions: Record<string, string> = {
    typescript: '.ts', python: '.py', rust: '.rs', zig: '.zig',
  };
  const ext      = extensions[lang];
  const fileName = lang === 'python'
    ? toSnake(agentName) + ext
    : agentName + ext;
  const destPath = path.join(agentsDir, fileName);

  if (fs.existsSync(destPath)) {
    logger.error(`Agent file "${fileName}" already exists.`);
    process.exit(1);
  }

  // ── Generate ───────────────────────────────────────────────────────────────
  const resolvedFn = templateFn ?? PLAIN_TEMPLATES[lang]!;
  const spinner    = ora(
    `Generating ${agentName} [${lang}${framework !== 'none' ? ` · ${framework}` : ''}]...`
  ).start();

  await fs.writeFile(destPath, resolvedFn(agentName, caps), 'utf8');
  spinner.succeed(`Agent created: agents/${fileName}`);

  // ── Post-create hints ──────────────────────────────────────────────────────
  logger.info(`Capabilities : ${caps.join(', ')}`);
  if (framework !== 'none') {
    logger.info(`Framework    : ${framework}`);
  }

  const hint = INSTALL_HINTS[framework]?.[lang];
  if (hint) {
    console.log('');
    console.log(chalk.cyan('  Framework dependencies:'));
    console.log(chalk.dim(`    ${hint}`));
    console.log('');

    // ── Auto-install ──────────────────────────────────────────────────────────
    let shouldInstall = options.yes ?? false;

    if (!shouldInstall) {
      const { install } = await inquirer.prompt([{
        type:    'confirm',
        name:    'install',
        message: `Install ${framework} dependencies now?`,
        default: true,
      }]);
      shouldInstall = install as boolean;
    }

    if (shouldInstall) {
      const installSpinner = ora(`Installing ${framework} dependencies...`).start();
      try {
        execSync(hint, { encoding: 'utf8', cwd: projectDir });
        installSpinner.succeed(`${framework} dependencies installed`);
      } catch (err: any) {
        installSpinner.fail('Dependency install failed');
        const output = (err.stdout || err.stderr || err.message || '').trim();
        if (output) console.log(chalk.red(`\n${output}`));
        console.log(chalk.yellow(`\n  Run manually:  ${hint}`));
      }
    } else {
      console.log(chalk.dim(`  Skipped. Run later:  ${hint}`));
    }
  }

  // ── x402 add-on hint ────────────────────────────────────────────────────────
  if (options.addon === 'x402') {
    const x402Snippet = lang === 'python'
      ? `\nfrom addons.x402 import X402ServerMixin, CapabilityPricing\n# Mixin: class ${agentName}(X402ServerMixin, IAgent):\n#   x402_pricing = { '${caps[0] ?? 'myCapability'}': CapabilityPricing.usdc_base(50, '0xMyWallet') }`
      : `\nimport { withX402Payment, usdcBase } from '../addons/x402';\n// const agent = withX402Payment(new ${agentName}(), { pricing: { '${caps[0] ?? 'myCapability'}': usdcBase(50, '0xMyWallet') } });`;
    console.log('');
    console.log(chalk.cyan('  x402 payment add-on enabled:'));
    console.log(chalk.dim(x402Snippet));
    console.log(chalk.dim('  Docs: docs/x402.md'));
  }

  // Some frameworks are Python-only — print a note if user asked for another language
  const PYTHON_ONLY_FRAMEWORKS: Framework[] = ['crewai', 'agno', 'smolagents'];
  if (PYTHON_ONLY_FRAMEWORKS.includes(framework) && lang !== 'python') {
    console.log('');
    console.log(chalk.yellow(
      `  Note: ${framework} only supports Python. A plain IAgent was generated for ${lang}.`
    ));
  }

  console.log('');
  console.log(chalk.dim('  Next steps:'));
  console.log(chalk.dim(`    1. Edit  agents/${fileName}  — fill in your implementations`));
  if (framework !== 'none') {
    console.log(chalk.dim(`    2. Register your agent: await ${agentName}.register_discovery()`));
  } else {
    console.log(chalk.dim(`    2. Register your agent: await ${agentName}.register_discovery()`));
  }
  console.log(chalk.dim(`    3. Query from another agent: registry.query('${caps[0] ?? 'yourCapability'}')`));
  if (options.addon === 'x402') {
    console.log(chalk.dim('    4. Add payment: docs/x402.md'));
  }
}
