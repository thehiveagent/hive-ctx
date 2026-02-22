import fs from "node:fs";
import path from "node:path";

type NativeModule = {
  HiveCtxEngine: new (storagePath: string, budgetTokens?: number) => {
    storagePath: string;
    budgetTokens?: number;
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

export type HiveCtxEngineOptions = {
  budgetTokens?: number;
};

export class HiveCtxEngine {
  private readonly inner: InstanceType<NativeModule["HiveCtxEngine"]>;

  constructor(storagePath: string, options: HiveCtxEngineOptions = {}) {
    this.inner = new native.HiveCtxEngine(storagePath, options.budgetTokens);
  }

  get storagePath(): string {
    return this.inner.storagePath;
  }

  get budgetTokens(): number | undefined {
    return this.inner.budgetTokens;
  }
}
