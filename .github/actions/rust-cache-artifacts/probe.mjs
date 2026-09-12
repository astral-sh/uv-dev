import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import {
  CacheService,
  GitHub,
  cacheVersion,
  downloadArchive,
} from "./index.mjs";

assert.equal(process.env.GITHUB_REPOSITORY, "astral-sh/uv-dev");
assert.equal(process.env.GITHUB_EVENT_NAME, "workflow_dispatch");
assert.notEqual(process.env.GITHUB_REF, "refs/heads/main");
const directory = process.env.INPUT_DIRECTORY;
const filename = path.join(directory, "cache.archive");
const service = new CacheService();
const paths = ["bin", ".crates.toml", ".crates2.json", "registry", "git"]
  .map((item) => path.join(os.homedir(), ".cargo", item))
  .concat(path.join(process.env.GITHUB_WORKSPACE, "target"));
const version = cacheVersion(paths, "zstd-without-long", "Linux");
if (process.env.INPUT_MODE === "export") {
  const entries = (await new GitHub().caches("astral-sh/uv-dev")).filter(
    (item) =>
      item.version === version &&
      /^v0-rust-.*-Linux-x64-[0-9a-f]{8}-[0-9a-f]{8}$/.test(item.key),
  );
  entries.sort((a, b) => a.size_in_bytes - b.size_in_bytes);
  assert.ok(entries.length, "No real default-branch cache to transfer");
  const entry = entries[0];
  const sha256 = await downloadArchive(
    service,
    entry.key,
    entry.version,
    entry.size_in_bytes,
    filename,
  );
  const key = `rust-cache-artifact-probe-${process.env.GITHUB_RUN_ID}-${crypto.createHash("sha256").update(entry.key).digest("hex").slice(0, 16)}`;
  fs.writeFileSync(
    path.join(directory, "probe.json"),
    JSON.stringify({
      key,
      sourceKey: entry.key,
      version,
      size: entry.size_in_bytes,
      sha256,
      paths,
    }),
  );
  console.log(
    `Exported real cache ${entry.key} (${entry.size_in_bytes} bytes)`,
  );
} else {
  assert.equal(process.env.INPUT_MODE, "import");
  const entry = JSON.parse(
    fs.readFileSync(path.join(directory, "probe.json"), "utf8"),
  );
  assert.deepEqual(entry.paths, paths);
  assert.equal(entry.version, version);
  assert.ok(
    entry.key.startsWith(
      `rust-cache-artifact-probe-${process.env.GITHUB_RUN_ID}-`,
    ),
  );
  assert.equal(fs.statSync(filename).size, entry.size);
  const hash = crypto.createHash("sha256");
  for await (const chunk of fs.createReadStream(filename)) hash.update(chunk);
  assert.equal(hash.digest("hex"), entry.sha256);
  const roots = paths.map((item) =>
    path.relative(process.env.GITHUB_WORKSPACE, item).replaceAll("\\", "/"),
  );
  const result = spawnSync(
    "python3",
    [
      new URL("validate-archive.py", import.meta.url).pathname,
      filename,
      "zstd-without-long",
      JSON.stringify(roots),
    ],
    { stdio: "inherit" },
  );
  assert.equal(result.status, 0);
  await service.save(filename, entry.key, version);
  const delimiter = crypto.randomUUID();
  fs.appendFileSync(
    process.env.GITHUB_OUTPUT,
    `key=${entry.key}\npaths<<${delimiter}\n${paths.join("\n")}\n${delimiter}\n`,
  );
}
