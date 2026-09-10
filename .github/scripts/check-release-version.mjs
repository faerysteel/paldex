#!/usr/bin/env node

import { appendFile, readFile } from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const releaseTag = process.env.RELEASE_TAG?.trim() || null;

async function readJson(relativePath) {
  const absolutePath = path.join(repoRoot, relativePath);
  try {
    return JSON.parse(await readFile(absolutePath, "utf8"));
  } catch (error) {
    throw new Error(`cannot read ${relativePath}: ${error.message}`);
  }
}

function readCargoVersion() {
  const result = spawnSync(
    "cargo",
    ["metadata", "--locked", "--no-deps", "--format-version", "1"],
    { cwd: repoRoot, encoding: "utf8" },
  );
  if (result.error) {
    throw new Error(`cannot run cargo metadata: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`cargo metadata failed: ${result.stderr.trim() || `exit ${result.status}`}`);
  }

  let metadata;
  try {
    metadata = JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`cargo metadata returned invalid JSON: ${error.message}`);
  }

  const appPackages = metadata.packages.filter((pkg) => pkg.name === "paldex-app");
  if (appPackages.length !== 1) {
    throw new Error(`expected one Cargo package named paldex-app, found ${appPackages.length}`);
  }
  return appPackages[0].version;
}

function validateTag(tag, version) {
  const match = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.exec(tag);
  if (!match) {
    throw new Error(
      `release tag ${JSON.stringify(tag)} is invalid; expected canonical stable vMAJOR.MINOR.PATCH without leading zeros or suffixes`,
    );
  }
  if (tag !== `v${version}`) {
    throw new Error(`release tag ${tag} does not match committed application version ${version}`);
  }
}

async function main() {
  const rootPackage = await readJson("package.json");
  const appPackage = await readJson("apps/paldex-app/package.json");
  const tauriConfig = await readJson("apps/paldex-app/src-tauri/tauri.conf.json");
  const versions = new Map([
    ["package.json", rootPackage.version],
    ["apps/paldex-app/package.json", appPackage.version],
    ["apps/paldex-app/src-tauri/tauri.conf.json", tauriConfig.version],
    ["apps/paldex-app/src-tauri/Cargo.toml", readCargoVersion()],
  ]);

  const distinctVersions = new Set(versions.values());
  if (distinctVersions.size !== 1) {
    const details = [...versions].map(([file, version]) => `${file}=${JSON.stringify(version)}`).join(", ");
    throw new Error(`application versions disagree: ${details}`);
  }

  const [version] = distinctVersions;
  if (typeof version !== "string" || /[\r\n]/.test(version)) {
    throw new Error(`application version is not a safe string: ${JSON.stringify(version)}`);
  }
  if (releaseTag) validateTag(releaseTag, version);

  const output = process.env.GITHUB_OUTPUT;
  if (output) {
    await appendFile(output, `version=${version}\ntag=${releaseTag ?? ""}\n`, "utf8");
  }
  console.log(
    releaseTag
      ? `Validated application version ${version} against release tag ${releaseTag}.`
      : `Validated application version ${version}.`,
  );
}

main().catch((error) => {
  console.error(`Release version validation failed: ${error.message}`);
  process.exitCode = 1;
});
