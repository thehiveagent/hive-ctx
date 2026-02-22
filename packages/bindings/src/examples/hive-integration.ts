import {
  ClassifierResultDto,
  HiveCtx,
  HiveCtxPlugin,
  PluginRetrieveResult,
} from "../index";

class QuickNotesPlugin implements HiveCtxPlugin {
  public name = "quick-notes";

  async retrieve(
    message: string,
    _weights: ClassifierResultDto,
  ): Promise<PluginRetrieveResult> {
    const content = `The last note about "${message}" is still in draft.`;
    return { content, tokens: 30 };
  }
}

async function runHiveAgent() {
  const hive = new HiveCtx({
    storagePath: "./data/hive-integration",
    budgetTokens: 320,
    profile: { team: "the hive", priority: "high" },
  });

  hive.use(new QuickNotesPlugin());

  await hive.episode("How should we replant the orchard?", "Let's rotate the east beds.");
  await hive.remember("Orchard rotation needs new lime trees.");

  const context = await hive.build("What can we do about the orchard?");
  console.log("-- Hive context --");
  console.log("System prompt:", context.systemPrompt);
  console.log("Tokens used:", context.tokenCount);
  console.log("Layers:", context.layers);
  console.log("Fingerprint mode:", context.fingerprintMode);
  console.log("Plugin contributions:", context.pluginContributions);
}

void runHiveAgent();
