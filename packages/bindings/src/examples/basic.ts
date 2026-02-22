import { HiveCtx, HiveCtxConfig } from "../index";

async function main() {
  const config: HiveCtxConfig = {
    storagePath: "./data/hive",
    budgetTokens: 400,
    model: "hive-01",
    profile: { name: "Casey", role: "gardener" },
  };

  const hive = new HiveCtx(config);
  const context = await hive.build("What's the status of the greenhouse project?");
  console.log("Compiled context:");
  console.log("System prompt:", context.systemPrompt);
  console.log("Tokens used:", context.tokenCount);
  console.log("Layers:", context.layers);
  console.log("Plugin contributions:", context.pluginContributions);
}

void main();
