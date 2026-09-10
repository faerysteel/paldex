#!/usr/bin/env node

import { randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, rm, rmdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build, createServer } from "vite";

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const fixtureDir = path.join(appRoot, "public", "__fixture__");
const sentinelName = `packaging-boundary-${randomUUID()}.txt`;
const sentinelPath = path.join(fixtureDir, sentinelName);
const sentinelContents = `preview-only:${randomUUID()}\n`;
const outDir = path.join(tmpdir(), `paldex-production-${randomUUID()}`);
let server;

try {
  await mkdir(fixtureDir, { recursive: true });
  await writeFile(sentinelPath, sentinelContents, { flag: "wx" });

  await build({
    root: appRoot,
    configFile: path.join(appRoot, "vite.config.ts"),
    logLevel: "warn",
    build: { outDir, emptyOutDir: true },
  });

  if (existsSync(path.join(outDir, "__fixture__"))) {
    throw new Error("production build copied the preview fixture directory");
  }

  server = await createServer({
    configFile: path.join(appRoot, "vite.preview.config.ts"),
    logLevel: "error",
    server: { host: "127.0.0.1", port: 0, strictPort: false },
  });
  await server.listen();

  const address = server.httpServer?.address();
  if (!address || typeof address === "string") {
    throw new Error("preview server did not expose a local TCP port");
  }

  const response = await globalThis.fetch(
    `http://127.0.0.1:${address.port}/__fixture__/${sentinelName}`,
  );
  const body = await response.text();
  if (!response.ok || body !== sentinelContents) {
    throw new Error(`preview server did not serve the fixture sentinel (${response.status})`);
  }

  globalThis.console.log(
    "Production excludes preview fixtures; the preview server still serves them.",
  );
} finally {
  if (server) await server.close();
  await rm(sentinelPath, { force: true });
  await rmdir(fixtureDir).catch((error) => {
    if (error.code !== "ENOTEMPTY" && error.code !== "ENOENT") throw error;
  });
  await rm(outDir, { recursive: true, force: true });
}
