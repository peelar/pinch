import { readFileSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";

const { version } = JSON.parse(readFileSync(new URL("../package.json", import.meta.url)));
if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error(`Unsupported app version: ${version}`);
const manifest = new URL("../Cargo.toml", import.meta.url);
const cargo = readFileSync(manifest, "utf8");
// Match only the workspace version, leaving dependency versions untouched.
const pattern = /(\[workspace\.package\]\s*\nversion = ")[^"]+(")/;
if (!pattern.test(cargo)) throw new Error("Workspace version not found");
const updated = cargo.replace(pattern, (_, prefix, suffix) => `${prefix}${version}${suffix}`);
if (process.argv.includes("--check")) {
  if (updated !== cargo) throw new Error("Cargo.toml version differs from package.json; run node scripts/sync-version.mjs");
  const metadata = JSON.parse(execFileSync("cargo", ["metadata", "--locked", "--no-deps", "--format-version", "1"], { encoding: "utf8" }));
  if (metadata.packages.some(pkg => metadata.workspace_members.includes(pkg.id) && pkg.version !== version)) {
    throw new Error("Cargo workspace versions differ");
  }
} else {
  writeFileSync(manifest, updated);
  execFileSync("cargo", ["metadata", "--no-deps", "--format-version", "1"], { stdio: "ignore" });
}
