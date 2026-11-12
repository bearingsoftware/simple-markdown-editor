// Launches a throwaway VS Code (downloaded into .vscode-test/, own user-data and extensions dirs, so the
// installed VS Code is untouched) and runs suite.cjs inside its extension host.
import { runTests } from "@vscode/test-electron";
import { fileURLToPath } from "node:url";
import { dirname, resolve, join } from "node:path";
import { mkdtempSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../..");
const scratch = mkdtempSync(join(tmpdir(), "sme-vscode-"));
mkdirSync(join(scratch, "ws"));
await runTests({
  version: process.env.SME_VSCODE_VERSION ?? "1.145.3",
  cachePath: join(root, ".vscode-test"),
  extensionDevelopmentPath: join(root, "extension"),
  extensionTestsPath: join(here, "suite.cjs"),
  launchArgs: [join(scratch, "ws"), "--disable-extensions", "--disable-workspace-trust", "--user-data-dir", join(scratch, "ud"), "--extensions-dir", join(scratch, "ext")],
  extensionTestsEnv: { SME_SCRATCH: scratch, SME_CSP_CONNECT: process.env.SME_CSP_CONNECT ?? "", SME_RESULTS: join(root, "results", process.env.SME_RESULTS_NAME ?? "vscode-e2e.json") },
});
