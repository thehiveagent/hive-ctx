import fs from "node:fs";
import path from "node:path";

type NativeModule = {
  HiveCtxEngine: new (storagePath: string, budgetTokens?: number) => {
    storagePath: string;
    budgetTokens?: number;
    classify_message(message: string): ClassifierResultDto;
    pipeline_build(
      message: string,
      user_profile: Record<string, string>,
      token_budget?: number | null,
    ): PipelineResultDto;
    graph_add_node(text: string, category?: string | null): Array<{ id: number }>;
    memory_store(text: string): number;
  };
};

function loadNative(): NativeModule {
  const envPath = process.env.HIVE_CTX_NATIVE_PATH;
  if (envPath) {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    return require(envPath) as NativeModule;
  }

  const addonDir = path.join(__dirname, "..");
  const directPath = path.join(addonDir, "hive_ctx.node");
  if (fs.existsSync(directPath)) {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    return require(directPath) as NativeModule;
  }

  const candidates = fs
    .readdirSync(addonDir)
    .filter((f) => f.startsWith("hive_ctx.") && f.endsWith(".node"))
    .sort();

  if (candidates.length > 0) {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    return require(path.join(addonDir, candidates[0])) as NativeModule;
  }

  throw new Error(
    `Failed to load native addon. Build it with "npm run build:native" (expected hive_ctx*.node in ${addonDir}).`,
  );
}

const native = loadNative();

export type ClassifierResultDto = {
  temporal_weight: number;
  personal_weight: number;
  technical_weight: number;
  emotional_weight: number;
  message_type: string;
  session_state: string;
};

export type PipelineLayersDto = {
  episodes: number;
  graph_nodes: number;
  fingerprint_entries: number;
  fingerprint_mode: string;
  included_layers: string[];
};

export type PipelineResultDto = {
  system_prompt: string;
  token_count: number;
  layers: PipelineLayersDto;
};

export interface HiveCtxConfig {
  storagePath: string;
  budgetTokens?: number;
  model?: string;
  profile?: Record<string, string>;
}

export type PluginRetrieveResult = {
  content: string;
  tokens: number;
};

export interface HiveCtxPlugin {
  name: string;
  retrieve(
    message: string,
    weights: ClassifierResultDto,
  ): Promise<PluginRetrieveResult>;
}

export interface PluginContribution {
  name: string;
  content: string;
  tokens: number;
}

export interface ContextResult {
  systemPrompt: string;
  tokenCount: number;
  fingerprintMode: string;
  layers: string[];
  pluginContributions: PluginContribution[];
}

const DEFAULT_TOKEN_BUDGET = 300;

export class HiveCtx {
  private readonly inner: InstanceType<NativeModule["HiveCtxEngine"]>;
  private readonly plugins = new Map<string, HiveCtxPlugin>();
  private readonly profile: Record<string, string>;
  private readonly budgetTokens?: number;

  constructor(private readonly config: HiveCtxConfig) {
    this.profile = config.profile ?? {};
    this.budgetTokens = config.budgetTokens;
    this.inner = new native.HiveCtxEngine(config.storagePath, config.budgetTokens);
  }

  public async build(message: string): Promise<ContextResult> {
    const classified = this.inner.classify_message(message);
    const pipeline = this.inner.pipeline_build(
      message,
      this.profile,
      this.budgetTokens ?? null,
    );
    const budgetLimit = this.budgetTokens ?? DEFAULT_TOKEN_BUDGET;
    const pluginBudget = Math.max(0, budgetLimit - pipeline.token_count);
    const pluginContext = await this.runPlugins(message, classified, pluginBudget);

    let prompt = pipeline.system_prompt;
    if (pluginContext.contributions.length > 0) {
      prompt +=
        "\n\n" +
        pluginContext.contributions
          .map((contribution) => `[${contribution.name}] ${contribution.content}`)
          .join("\n");
    }

    return {
      systemPrompt: prompt,
      tokenCount: pipeline.token_count + pluginContext.tokensUsed,
      fingerprintMode: pipeline.layers.fingerprint_mode,
      layers: pipeline.layers.included_layers,
      pluginContributions: pluginContext.contributions,
    };
  }

  public async remember(fact: string): Promise<void> {
    const sanitized = fact.trim();
    if (sanitized.length === 0) {
      return;
    }
    this.inner.graph_add_node(sanitized);
  }

  public async episode(message: string, response: string): Promise<void> {
    const trimmedMessage = message.trim();
    const trimmedResponse = response.trim();
    if (!trimmedMessage && !trimmedResponse) {
      return;
    }
    const payload = [trimmedMessage, trimmedResponse].filter(Boolean).join(" || ");
    this.inner.memory_store(payload);
  }

  public use(plugin: HiveCtxPlugin): void {
    this.plugins.set(plugin.name, plugin);
  }

  private async runPlugins(
    message: string,
    weights: ClassifierResultDto,
    budget: number,
  ): Promise<{ contributions: PluginContribution[]; tokensUsed: number }> {
    const contributions: PluginContribution[] = [];
    let tokensUsed = 0;
    let remaining = budget;

    for (const plugin of this.plugins.values()) {
      if (remaining <= 0) {
        break;
      }
      const candidate = await plugin.retrieve(message, weights);
      const snippet = candidate.content.trim();
      if (!snippet || candidate.tokens <= 0 || candidate.tokens > remaining) {
        continue;
      }
      contributions.push({
        name: plugin.name,
        content: snippet,
        tokens: candidate.tokens,
      });
      tokensUsed += candidate.tokens;
      remaining -= candidate.tokens;
    }

    return { contributions, tokensUsed };
  }
}
