import assert from "node:assert/strict";
import crypto from "node:crypto";
import { test } from "node:test";
import {
  buildExportIndex,
  cacheVersion,
  destinationPaths,
  parseCacheLog,
  validateEntry,
  validateSourceRun,
} from "./index.mjs";

const sha = "8e70deb71b410d4bcf80fabdd88e8aa43a2e5878";
const sourceRun = "34694089787";
const workspace = "/home/runner/work/uv/uv";
const paths = ["bin", ".crates.toml", ".crates2.json", "registry", "git"]
  .map((name) => `/home/runner/.cargo/${name}`)
  .concat(`${workspace}/target`);
const key =
  "v1-rust-workspace-build-binary-linux-libc-Linux-x64-98ed5745-85f4849a";
const version =
  "1a74268c6d70ab6a116c1be58ca62719b03d3c66ef978106406266cf8340454f";
const destinationVersion =
  "ea8d493721c963b2cc85e216814188f384e64799b41c9db5512c8854fba6d02d";
const run = {
  id: Number(sourceRun),
  repository: { full_name: "astral-sh/uv" },
  head_repository: { full_name: "astral-sh/uv" },
  path: ".github/workflows/ci.yml",
  event: "push",
  head_branch: "main",
  head_sha: sha,
  status: "completed",
  conclusion: "success",
};
const log = `Run Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6
Working directory is '${workspace}'
##[group]Cache Configuration
Cache Provider:
    github
Workspaces:
    ${workspace}
Cache Paths:
${paths.map((item) => `    ${item}`).join("\n")}
Restore Key:
    ${key.slice(0, -9)}
Cache Key:
    ${key}
.. Prefix:
    unused
##[endgroup]`;

function entry(overrides = {}) {
  return {
    sourceRepository: "astral-sh/uv",
    sourceRun,
    sourceSha: sha,
    sourceJob: 1,
    sourceWorkspace: workspace,
    sourcePaths: paths,
    key,
    version,
    destinationVersion,
    platform: "Linux",
    architecture: "x64",
    compression: "zstd-without-long",
    size: 1234,
    artifact: `rust-cache-${crypto.createHash("sha256").update(`${key}\0${version}`).digest("hex")}`,
    ...overrides,
  };
}

test("matches real source and destination cache versions", () => {
  assert.equal(cacheVersion(paths, "zstd-without-long", "Linux"), version);
  const destination = destinationPaths(workspace, paths, "Linux");
  assert.equal(
    destination.paths.at(-1),
    "/home/runner/work/uv-dev/uv-dev/target",
  );
  assert.equal(
    cacheVersion(destination.paths, "zstd-without-long", "Linux"),
    destinationVersion,
  );
  assert.deepEqual(validateEntry(entry()).relativePaths, [
    "../../../.cargo/bin",
    "../../../.cargo/.crates.toml",
    "../../../.cargo/.crates2.json",
    "../../../.cargo/registry",
    "../../../.cargo/git",
    "target",
  ]);
});

test("keeps Windows dev-drive paths unchanged", () => {
  const checkout = "D:\\a\\uv\\uv";
  const windows = ["bin", ".crates.toml", ".crates2.json", "registry", "git"]
    .map((name) => `D:\\.cargo\\${name}`)
    .concat("D:\\uv\\target");
  assert.deepEqual(
    destinationPaths(checkout, windows, "Windows_NT").paths,
    windows,
  );
  assert.equal(
    cacheVersion(windows, "zstd-without-long", "Windows_NT"),
    "ef22b121d4690ad7b4e1de2d80c4e2e1ea1eb6b6b03bdccaa62ed6044a53a165",
  );
});

test("rejects mismatched identities and unrecognized paths", () => {
  for (const overrides of [
    { key: "other" },
    { sourceSha: "main" },
    { sourceRepository: "contributor/uv" },
    { version: "0".repeat(64) },
    { destinationVersion: version },
    { size: -1 },
    { sourcePaths: [...paths.slice(0, -1), "/etc"] },
  ]) {
    assert.throws(() => validateEntry(entry(overrides)));
  }
});

test("parses the pinned cache log, including duplicate post-step output", () => {
  const timestamped = log
    .split("\n")
    .map((line) => `2026-09-12T12:36:23.2661160Z ${line}`)
    .join("\n");
  assert.deepEqual(parseCacheLog(`${timestamped}\n${timestamped}`), {
    sourceWorkspace: workspace,
    sourcePaths: paths,
    key,
  });
  assert.throws(() =>
    parseCacheLog(log.replace("github\nWorkspaces:", "warpbuild\nWorkspaces:")),
  );
  assert.throws(() =>
    parseCacheLog(
      log.replace("6323deb102c322ba6fcbdcafc7e3dddab59af2b6", "main"),
    ),
  );
  assert.throws(() =>
    parseCacheLog(`${log}\nWorking directory is '/other/uv/uv'`),
  );
});

test("accepts only successful canonical main CI", () => {
  validateSourceRun(run, sha);
  for (const overrides of [
    { event: "pull_request" },
    { head_branch: "feature" },
    { head_sha: "0".repeat(40) },
    { conclusion: "failure" },
    { path: ".github/workflows/other.yml" },
    { head_repository: { full_name: "contributor/uv" } },
  ]) {
    assert.throws(() => validateSourceRun({ ...run, ...overrides }, sha));
  }
});

test("exports only current-run cache keys missing from the mirror", async () => {
  const cache = { key, version, ref: "refs/heads/main", size_in_bytes: 1234 };
  let mirror = [];
  const github = {
    get: async () => run,
    pages: async () => [
      { id: 1, name: "build-dev-binaries / linux libc", conclusion: "success" },
      { id: 2, name: "test / failed", conclusion: "failure" },
      { id: 3, name: "unknown / untrusted", conclusion: "success" },
    ],
    caches: async (repository) =>
      repository === "astral-sh/uv"
        ? [cache, { ...cache, ref: "refs/pull/1/merge" }]
        : mirror,
    jobLog: async () => log,
  };
  const index = await buildExportIndex(github, sourceRun, sha);
  assert.deepEqual(index.entries, [entry()]);
  mirror = [{ key, version: destinationVersion }];
  assert.deepEqual(
    (await buildExportIndex(github, sourceRun, sha)).entries,
    [],
  );
});
