/**
 * CrewAI → Borgkit Plugin (TypeScript) — HTTP Bridge
 * ─────────────────────────────────────────────────────────────────────────────
 * Bridges a running CrewAI HTTP service into the Borgkit mesh.
 *
 * CrewAI is Python-native; this plugin calls it over HTTP so TypeScript agents
 * can invoke CrewAI crews without shipping Python bindings.
 *
 * ── Expected service endpoints ────────────────────────────────────────────────
 *
 *   GET  /capabilities
 *     → [{ name: string, description: string, parameters?: Record }]
 *
 *   POST /kickoff
 *     Body:     { capability?: string, inputs: Record, task?: string }
 *     Response: { result: string, status: "success"|"error", error?: string }
 *
 * ── Serving CrewAI over HTTP ──────────────────────────────────────────────────
 *
 * Option A — zero-dependency FastAPI wrapper (drop next to your crew):
 *
 *   # serve_crew.py
 *   from fastapi import FastAPI
 *   from my_crew import my_crew            # your Crew instance
 *
 *   app = FastAPI()
 *
 *   @app.get("/capabilities")
 *   def caps():
 *       return [{"name": "kickoff", "description": "Run the crew on a task"}]
 *
 *   @app.post("/kickoff")
 *   async def kickoff(body: dict):
 *       result = my_crew.kickoff(inputs=body.get("inputs", {}))
 *       return {"result": str(result), "status": "success"}
 *
 *   # uvicorn serve_crew:app --port 8000
 *
 * Option B — borgkit-crewai Python adapter (auto-generates endpoints):
 *
 *   pip install borgkit-crewai
 *   borgkit-crewai serve --crew my_package:my_crew --port 8000
 *
 * ── Usage ─────────────────────────────────────────────────────────────────────
 *
 *   import { CrewAIPlugin } from './plugins/CrewAIPlugin';
 *
 *   const plugin = new CrewAIPlugin({
 *     agentId:    'borgkit://agent/researcher',
 *     name:       'ResearchAgent',
 *     version:    '1.0.0',
 *     owner:      '0xYourWallet',
 *     serviceUrl: 'http://localhost:8000',
 *     tags:       ['research', 'crewai'],
 *   });
 *
 *   // Eagerly fetch capabilities from the service (optional but recommended):
 *   await plugin.fetchCapabilities();
 *
 *   const agent = plugin.wrap(plugin.createService());
 *   await agent.registerDiscovery();
 */

import { AgentRequest }        from '../interfaces/IAgentRequest';
import { AgentResponse }       from '../interfaces/IAgentResponse';
import { BorgkitPlugin, PluginConfig, CapabilityDescriptor } from './IPlugin';

// ── Config ────────────────────────────────────────────────────────────────────

export interface CrewAIPluginConfig extends PluginConfig {
  /** Base URL of the running CrewAI HTTP service (default: 'http://localhost:8000') */
  serviceUrl?: string;
  /** Optional Bearer token sent as Authorization header */
  apiKey?: string;
  /** HTTP request timeout in milliseconds (default: 120 000) */
  timeoutMs?: number;
  /**
   * Static capability list — skips GET /capabilities entirely.
   * Useful when the service does not expose a capabilities endpoint.
   */
  staticCapabilities?: Array<{
    name:         string;
    description:  string;
    parameters?:  Record<string, unknown>;
  }>;
  /** POST path for crew execution (default: '/kickoff') */
  kickoffPath?: string;
  /** GET path for capability discovery (default: '/capabilities') */
  capabilitiesPath?: string;
}

// ── Service handle ────────────────────────────────────────────────────────────

/**
 * Opaque token representing a live CrewAI HTTP service.
 * Create one via `plugin.createService()`.
 */
export interface CrewAIService {
  serviceUrl:       string;
  apiKey?:          string;
  timeoutMs:        number;
  kickoffPath:      string;
  capabilitiesPath: string;
}

// ── Wire types ────────────────────────────────────────────────────────────────

interface CrewAIInput {
  capability?: string;
  task?:       string;
  inputs:      Record<string, unknown>;
}

interface CrewAIOutput {
  result?:  string;
  output?:  string;
  status?:  string;
  error?:   string;
  [key: string]: unknown;
}

interface RemoteCapability {
  name:        string;
  description: string;
  parameters?: Record<string, unknown>;
}

// ── Plugin ────────────────────────────────────────────────────────────────────

export class CrewAIPlugin extends BorgkitPlugin<CrewAIService, CrewAIInput, CrewAIOutput> {
  private readonly crewConfig: {
    serviceUrl:       string;
    apiKey?:          string;
    timeoutMs:        number;
    kickoffPath:      string;
    capabilitiesPath: string;
    staticCapabilities?: CrewAIPluginConfig['staticCapabilities'];
  };

  /** Remote capabilities fetched via GET /capabilities (cached after first fetch). */
  private _remoteCapabilities?: CapabilityDescriptor[];

  constructor(config: CrewAIPluginConfig) {
    super(config);
    this.crewConfig = {
      serviceUrl:         config.serviceUrl        ?? 'http://localhost:8000',
      apiKey:             config.apiKey,
      timeoutMs:          config.timeoutMs          ?? 120_000,
      kickoffPath:        config.kickoffPath        ?? '/kickoff',
      capabilitiesPath:   config.capabilitiesPath   ?? '/capabilities',
      staticCapabilities: config.staticCapabilities,
    };
  }

  // ── Factory ────────────────────────────────────────────────────────────────

  /** Build the service handle to pass to `plugin.wrap()`. */
  createService(): CrewAIService {
    return {
      serviceUrl:       this.crewConfig.serviceUrl,
      apiKey:           this.crewConfig.apiKey,
      timeoutMs:        this.crewConfig.timeoutMs,
      kickoffPath:      this.crewConfig.kickoffPath,
      capabilitiesPath: this.crewConfig.capabilitiesPath,
    };
  }

  // ── Capability extraction ──────────────────────────────────────────────────

  /**
   * Resolve capabilities using this priority:
   *   1. `staticCapabilities` from config
   *   2. Cached capabilities from a prior `fetchCapabilities()` call
   *   3. Single `'invoke'` fallback (lazy — fetched on first HTTP call if needed)
   */
  extractCapabilities(_service: CrewAIService): CapabilityDescriptor[] {
    if (this.crewConfig.staticCapabilities?.length) {
      return this.crewConfig.staticCapabilities.map(c => this._toDescriptor(c));
    }
    if (this._remoteCapabilities?.length) {
      return this._remoteCapabilities;
    }
    return [{
      name:        'invoke',
      description: this.config.description ?? 'Invoke the CrewAI crew',
      nativeName:  '__crew__',
      tags:        this.config.tags ?? [],
    }];
  }

  /**
   * Eagerly fetch capabilities from the service's GET /capabilities endpoint.
   *
   * Call this before `plugin.wrap()` to populate the capability list so that
   * discovery announces all individual crew tools.
   *
   * @example
   * ```ts
   * await plugin.fetchCapabilities();
   * const agent = plugin.wrap(plugin.createService());
   * ```
   */
  async fetchCapabilities(): Promise<CapabilityDescriptor[]> {
    const svc = this.createService();
    const url = `${svc.serviceUrl.replace(/\/$/, '')}${svc.capabilitiesPath}`;
    const res = await this._doFetch(svc, url, 'GET');
    if (!res.ok) {
      throw new Error(`GET ${url} returned ${res.status}`);
    }
    const data = await res.json() as unknown;
    const list = Array.isArray(data) ? (data as RemoteCapability[]) : [];
    this._remoteCapabilities = list.map(c => this._toDescriptor(c));
    return this._remoteCapabilities;
  }

  private _toDescriptor(c: RemoteCapability): CapabilityDescriptor {
    return {
      name:        c.name,
      description: c.description,
      nativeName:  c.name,
      tags:        this.config.tags ?? [],
      inputSchema: c.parameters
        ? { type: 'object', properties: c.parameters }
        : undefined,
    };
  }

  // ── Request translation ────────────────────────────────────────────────────

  translateRequest(req: AgentRequest, descriptor: CapabilityDescriptor): CrewAIInput {
    const payload = req.payload as Record<string, unknown>;
    return {
      capability: descriptor.nativeName === '__crew__' ? undefined : descriptor.nativeName,
      task:       (payload['task'] ?? payload['query'] ?? payload['input']) as string | undefined,
      inputs:     payload,
    };
  }

  // ── Response translation ───────────────────────────────────────────────────

  translateResponse(output: CrewAIOutput, requestId: string): AgentResponse {
    if (output.status === 'error' || output.error) {
      return {
        requestId,
        status:       'error',
        errorMessage: output.error ?? JSON.stringify(output),
        timestamp:    Date.now(),
      };
    }
    const content = output.result ?? output.output ?? JSON.stringify(output);
    return {
      requestId,
      status:    'success',
      result:    { content, raw: output },
      timestamp: Date.now(),
    };
  }

  // ── Native invocation ──────────────────────────────────────────────────────

  async invokeNative(
    service:    CrewAIService,
    _d:         CapabilityDescriptor,
    input:      CrewAIInput,
  ): Promise<CrewAIOutput> {
    const url = `${service.serviceUrl.replace(/\/$/, '')}${service.kickoffPath}`;
    const res = await this._doFetch(service, url, 'POST', JSON.stringify(input));

    if (!res.ok) {
      const text = await res.text().catch(() => '');
      throw new Error(`CrewAI service returned HTTP ${res.status}: ${text}`);
    }

    return res.json() as Promise<CrewAIOutput>;
  }

  // ── HTTP helper ────────────────────────────────────────────────────────────

  private async _doFetch(
    service: CrewAIService,
    url:     string,
    method:  'GET' | 'POST',
    body?:   string,
  ): Promise<Response> {
    const headers: Record<string, string> = { Accept: 'application/json' };
    if (body)            headers['Content-Type'] = 'application/json';
    if (service.apiKey)  headers['Authorization'] = `Bearer ${service.apiKey}`;

    const controller = new AbortController();
    const timer      = setTimeout(() => controller.abort(), service.timeoutMs);

    try {
      const res = await fetch(url, { method, headers, body, signal: controller.signal });
      clearTimeout(timer);
      return res;
    } catch (err) {
      clearTimeout(timer);
      const isTimeout = err instanceof Error && err.name === 'AbortError';
      throw isTimeout
        ? new Error(`CrewAI service timed out after ${service.timeoutMs} ms`)
        : err;
    }
  }
}

// ── Convenience helper ────────────────────────────────────────────────────────

/**
 * Wrap a CrewAI HTTP service in a single call.
 *
 * @example
 * ```ts
 * const agent = await wrapCrewAI({
 *   agentId:    'borgkit://agent/writer',
 *   name:       'WriterCrew',
 *   version:    '1.0.0',
 *   owner:      '0xYourWallet',
 *   serviceUrl: 'http://localhost:8000',
 * });
 * await agent.registerDiscovery();
 * ```
 */
export async function wrapCrewAI(
  config: CrewAIPluginConfig,
  fetchCaps = true,
): Promise<ReturnType<CrewAIPlugin['wrap']>> {
  const plugin = new CrewAIPlugin(config);
  if (fetchCaps) {
    await plugin.fetchCapabilities().catch(() => { /* fall back to 'invoke' */ });
  }
  return plugin.wrap(plugin.createService());
}
