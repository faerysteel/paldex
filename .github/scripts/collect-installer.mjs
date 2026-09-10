#!/usr/bin/env node

import { appendFile, copyFile, mkdir, readdir } from "node:fs/promises";
import path from "node:path";

const required = [
  "EXPECTED_DIR",
  "EXTENSION",
  "PLATFORM",
  "ARCHITECTURE",
  "VERSION",
  "GITHUB_OUTPUT",
];
for (const name of required) {
  if (!process.env[name]) throw new Error(`${name} is required`);
}

async function regularFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await regularFiles(entryPath)));
    else if (entry.isFile()) files.push(entryPath);
    else throw new Error(`unexpected non-file installer output: ${entryPath}`);
  }
  return files;
}

const expectedDir = path.resolve(process.env.EXPECTED_DIR);
const files = await regularFiles(expectedDir);
const installerExtensions = new Set([".appimage", ".deb", ".dmg", ".exe", ".msi", ".rpm"]);
const installers = files.filter((file) =>
  installerExtensions.has(path.extname(file).toLowerCase()),
);
if (
  installers.length !== 1 ||
  path.extname(installers[0]).toLowerCase() !== process.env.EXTENSION
) {
  const found = installers.length
    ? installers.map((file) => path.relative(expectedDir, file)).join(", ")
    : "none";
  throw new Error(
    `expected exactly one ${process.env.EXTENSION} installer in ${expectedDir}; found ${found}`,
  );
}

const suffix = process.env.PLATFORM === "windows" ? "-setup.exe" : ".dmg";
const filename = `Paldex-${process.env.VERSION}-${process.env.PLATFORM}-${process.env.ARCHITECTURE}${suffix}`;
const outputDir = path.resolve("release-assets");
const outputPath = path.join(outputDir, filename);
await mkdir(outputDir, { recursive: true });
await copyFile(installers[0], outputPath);

const fixturePath = path.resolve("apps/paldex-app/dist/__fixture__");
try {
  await readdir(fixturePath);
  throw new Error(`production frontend contains forbidden preview fixtures: ${fixturePath}`);
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}

await appendFile(
  process.env.GITHUB_OUTPUT,
  `path=${outputPath}\nfilename=${filename}\n`,
  "utf8",
);
console.log(`Collected installer ${filename}.`);
