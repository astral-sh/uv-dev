import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import { Readable, Transform } from "node:stream";
import { pipeline } from "node:stream/promises";
import { spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";

const SOURCE = "astral-sh/uv";
const DESTINATION = "astral-sh/uv-dev";
const REF = "refs/heads/main";
const CACHE_ACTION = "6323deb102c322ba6fcbdcafc7e3dddab59af2b6";
const PRODUCERS = new Set([
  "check-lint",
  "check-docs",
  "check-publish",
  "check-generated-files",
  "test",
  "test-windows-trampolines",
  "build-dev-binaries",
]);
const COMPRESSIONS = ["zstd-without-long", "zstd", "gzip"];
const KEY =
  /^(?:v0-rust|v1-rust-workspace)-[A-Za-z0-9_-]+-(Linux|Darwin|Windows_NT)-(x64|arm64)-[0-9a-f]{8}-[0-9a-f]{8}$/;
const SHA = /^[0-9a-f]{40}$/;
const VERSION = /^[0-9a-f]{64}$/;
const MAX_CACHE_SIZE = 10 * 1024 ** 3;
const SERVICE = "github.actions.results.api.v1.CacheService";

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function digest(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

function wireField(value, name) {
  return (
    value[name] ??
    value[name.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`)]
  );
}

function input(name) {
  return process.env[`INPUT_${name.toUpperCase()}`] || "";
}

function output(name, value) {
  check(!String(value).includes("\n"), "Multiline action output");
  fs.appendFileSync(process.env.GITHUB_OUTPUT, `${name}=${value}\n`);
}

function writeJson(filename, value) {
  fs.mkdirSync(path.dirname(filename), { recursive: true });
  fs.writeFileSync(filename, `${JSON.stringify(value)}\n`);
}

function safeUrl(value) {
  const url = new URL(value);
  check(
    url.protocol === "https:" && !url.username && !url.password,
    "Invalid HTTPS URL",
  );
  return url;
}

async function request(url, options = {}) {
  for (let attempt = 0; attempt < 4; attempt++) {
    let response;
    try {
      response = await fetch(safeUrl(url), { redirect: "error", ...options });
    } catch {
      if (attempt === 3) throw new Error("HTTP request failed");
    }
    if (response?.ok) return response;
    if (response && response.status < 500 && response.status !== 429) {
      throw new Error(`HTTP request failed with status ${response.status}`);
    }
    if (attempt === 3)
      throw new Error(
        `HTTP request failed with status ${response?.status || "unknown"}`,
      );
    await new Promise((resolve) => setTimeout(resolve, 1000 * 2 ** attempt));
  }
}

export class GitHub {
  constructor(token = process.env.GH_TOKEN || "") {
    this.token = token;
  }

  async get(endpoint, { publicOnly = false } = {}) {
    check(endpoint.startsWith("repos/"), "Invalid GitHub API path");
    const headers = {
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2026-03-10",
    };
    if (this.token && !publicOnly)
      headers.Authorization = `Bearer ${this.token}`;
    return (
      await request(`https://api.github.com/${endpoint}`, { headers })
    ).json();
  }

  async pages(endpoint, field, options) {
    const result = [];
    for (let page = 1; page <= 100; page++) {
      const separator = endpoint.includes("?") ? "&" : "?";
      const response = await this.get(
        `${endpoint}${separator}per_page=100&page=${page}`,
        options,
      );
      const values = response[field];
      check(Array.isArray(values), "Invalid paginated GitHub response");
      result.push(...values);
      if (values.length < 100) return result;
    }
    throw new Error("GitHub pagination limit exceeded");
  }

  async jobLog(job) {
    const url = `https://api.github.com/repos/${SOURCE}/actions/jobs/${job}/logs`;
    const response = await fetch(url, {
      redirect: "manual",
      headers: {
        Authorization: `Bearer ${this.token}`,
        Accept: "application/vnd.github+json",
      },
    });
    const log =
      response.status === 302
        ? await request(response.headers.get("location"))
        : response;
    check(log.ok, "Could not download source job log");
    return log.text();
  }

  caches(repository, options) {
    return this.pages(
      `repos/${repository}/actions/caches?ref=${encodeURIComponent(REF)}`,
      "actions_caches",
      options,
    );
  }
}

// Match @actions/cache's getCacheVersion. The archive's paths are a separate
// part of its identity from rust-cache's public key.
export function cacheVersion(paths, compression, platform) {
  check(COMPRESSIONS.includes(compression), "Unsupported cache compression");
  return digest(
    [
      ...paths,
      compression,
      ...(platform === "Windows_NT" ? ["windows-only"] : []),
      "1.0",
    ].join("|"),
  );
}

function pathApi(platform) {
  return platform === "Windows_NT" ? path.win32 : path.posix;
}

function relativePaths(workspace, paths, platform) {
  const api = pathApi(platform);
  return paths.map((item) =>
    (api.relative(workspace, item) || ".").replaceAll("\\", "/"),
  );
}

function inside(api, root, item) {
  const relative = api.relative(root, item);
  return (
    relative === "" ||
    (!relative.startsWith(`..${api.sep}`) &&
      relative !== ".." &&
      !api.isAbsolute(relative))
  );
}

export function destinationPaths(workspace, paths, platform) {
  const api = pathApi(platform);
  check(
    api.basename(workspace) === "uv" &&
      api.basename(api.dirname(workspace)) === "uv",
    "Unexpected source checkout path",
  );
  const destination = api.join(
    api.dirname(api.dirname(workspace)),
    "uv-dev",
    "uv-dev",
  );
  const mapped = paths.map((item) =>
    inside(api, workspace, item)
      ? api.join(destination, api.relative(workspace, item))
      : item,
  );
  assert.deepEqual(
    relativePaths(workspace, paths, platform),
    relativePaths(destination, mapped, platform),
    "Cache archive paths are not portable",
  );
  return { workspace: destination, paths: mapped };
}

function validatePaths(workspace, paths, platform) {
  const api = pathApi(platform);
  check(
    Array.isArray(paths) && [3, 6].includes(paths.length),
    "Unexpected Rust cache path count",
  );
  check(
    paths.every(
      (item) =>
        typeof item === "string" &&
        api.isAbsolute(item) &&
        !/[\r\n\0]/.test(item),
    ),
    "Invalid Rust cache path",
  );
  const registry = paths.length === 6 ? 3 : 0;
  const cargo = api.dirname(paths[registry]);
  check(api.basename(cargo) === ".cargo", "Unexpected Cargo home");
  const expected = [
    ...(registry ? ["bin", ".crates.toml", ".crates2.json"] : []),
    "registry",
    "git",
  ].map((item) => api.join(cargo, item));
  assert.deepEqual(
    paths.slice(0, -1),
    expected,
    "Unexpected Cargo cache paths",
  );
  const allowedTargets = [
    api.join(workspace, "target"),
    api.join(workspace, "target", "hawk"),
  ];
  if (platform === "Windows_NT") {
    const drive = api.parse(cargo).root;
    allowedTargets.push(
      api.join(drive, "uv", "target"),
      api.join(drive, "uv", "crates", "uv-trampoline", "target"),
    );
  }
  check(
    allowedTargets.includes(paths.at(-1)),
    "Unexpected Rust target directory",
  );
}

export function validateEntry(entry) {
  check(
    entry &&
      entry.sourceRepository === SOURCE &&
      SHA.test(entry.sourceSha) &&
      /^\d+$/.test(String(entry.sourceRun)),
    "Invalid cache source identity",
  );
  const match = KEY.exec(entry.key);
  check(
    match && match[1] === entry.platform && match[2] === entry.architecture,
    "Invalid Rust cache key",
  );
  check(VERSION.test(entry.version), "Invalid cache version");
  check(
    Number.isSafeInteger(entry.size) &&
      entry.size > 0 &&
      entry.size <= MAX_CACHE_SIZE,
    "Invalid cache size",
  );
  validatePaths(entry.sourceWorkspace, entry.sourcePaths, entry.platform);
  check(
    cacheVersion(entry.sourcePaths, entry.compression, entry.platform) ===
      entry.version,
    "Source cache version mismatch",
  );
  const destination = destinationPaths(
    entry.sourceWorkspace,
    entry.sourcePaths,
    entry.platform,
  );
  check(
    cacheVersion(destination.paths, entry.compression, entry.platform) ===
      entry.destinationVersion,
    "Destination cache version mismatch",
  );
  check(
    entry.artifact === `rust-cache-${digest(`${entry.key}\0${entry.version}`)}`,
    "Invalid cache artifact name",
  );
  return {
    ...destination,
    relativePaths: relativePaths(
      entry.sourceWorkspace,
      entry.sourcePaths,
      entry.platform,
    ),
  };
}

export function parseCacheLog(log) {
  check(
    log.includes(`Swatinem/rust-cache@${CACHE_ACTION}`),
    "Unexpected Rust cache action version",
  );
  const lines = log
    .replace(/\x1b\[[0-9;]*m/g, "")
    .split(/\r?\n/)
    .map((line) => line.replace(/^\d{4}-\d\d-\d\dT[\d:.]+Z\s?/, ""));
  const workspaces = [
    ...new Set(
      lines.flatMap(
        (line) => /^Working directory is '([^']+)'$/.exec(line)?.slice(1) || [],
      ),
    ),
  ];
  check(workspaces.length === 1, "Ambiguous source checkout directory");
  const records = [];
  for (let start = 0; start < lines.length; start++) {
    if (lines[start] !== "##[group]Cache Configuration") continue;
    const end = lines.indexOf("##[endgroup]", start + 1);
    check(end !== -1, "Incomplete Rust cache configuration");
    const group = lines.slice(start + 1, end);
    const values = (from, to) => {
      const left = group.indexOf(from);
      const right = group.indexOf(to);
      check(left !== -1 && right > left, "Unknown Rust cache log format");
      return group
        .slice(left + 1, right)
        .map((line) => line.trim())
        .filter(Boolean);
    };
    assert.deepEqual(values("Cache Provider:", "Workspaces:"), ["github"]);
    const paths = values("Cache Paths:", "Restore Key:");
    const keys = values("Cache Key:", ".. Prefix:");
    check(keys.length === 1, "Ambiguous Rust cache key");
    records.push({
      sourceWorkspace: workspaces[0],
      sourcePaths: paths,
      key: keys[0],
    });
    start = end;
  }
  const unique = [
    ...new Map(records.map((entry) => [JSON.stringify(entry), entry])).values(),
  ];
  check(unique.length === 1, "Expected one Rust cache configuration per job");
  return unique[0];
}

export function validateSourceRun(run, sha, { completed = true } = {}) {
  check(
    run.repository?.full_name === SOURCE &&
      run.head_repository?.full_name === SOURCE,
    "Unexpected source repository",
  );
  check(
    run.path === ".github/workflows/ci.yml" &&
      run.event === "push" &&
      run.head_branch === "main" &&
      run.head_sha === sha,
    "Unexpected source CI run",
  );
  if (completed)
    check(
      run.status === "completed" && run.conclusion === "success",
      "Source CI has not succeeded",
    );
}

export async function buildExportIndex(
  github,
  runId,
  sha,
  { skipExisting = true, completed = false } = {},
) {
  check(SHA.test(sha) && /^\d+$/.test(String(runId)), "Invalid source run");
  validateSourceRun(
    await github.get(`repos/${SOURCE}/actions/runs/${runId}`),
    sha,
    { completed },
  );
  const [jobs, caches, mirror] = await Promise.all([
    github.pages(
      `repos/${SOURCE}/actions/runs/${runId}/jobs?filter=latest`,
      "jobs",
    ),
    github.caches(SOURCE),
    skipExisting ? github.caches(DESTINATION, { publicOnly: true }) : [],
  ]);
  const existing = new Set(
    mirror.map((entry) => `${entry.key}\0${entry.version}`),
  );
  const entries = new Map();
  for (const job of jobs) {
    if (
      job.conclusion !== "success" ||
      !PRODUCERS.has(job.name.split(" / ")[0])
    )
      continue;
    const log = await github.jobLog(job.id);
    if (!log.includes(`Swatinem/rust-cache@${CACHE_ACTION}`)) continue;
    const configuration = parseCacheLog(log);
    const match = KEY.exec(configuration.key);
    check(match, "Unknown Rust cache key format");
    const [, platform, architecture] = match;
    const candidates = caches.filter(
      (cache) => cache.ref === REF && cache.key === configuration.key,
    );
    const matches = COMPRESSIONS.flatMap((compression) =>
      candidates
        .filter(
          (cache) =>
            cache.version ===
            cacheVersion(configuration.sourcePaths, compression, platform),
        )
        .map((cache) => ({ cache, compression })),
    );
    if (matches.length === 0) continue; // Eviction or a best-effort cache save can leave no entry.
    check(matches.length === 1, "Ambiguous source cache version");
    const { cache, compression } = matches[0];
    const destination = destinationPaths(
      configuration.sourceWorkspace,
      configuration.sourcePaths,
      platform,
    );
    const entry = {
      ...configuration,
      sourceRepository: SOURCE,
      sourceRun: String(runId),
      sourceSha: sha,
      sourceJob: job.id,
      platform,
      architecture,
      compression,
      version: cache.version,
      destinationVersion: cacheVersion(
        destination.paths,
        compression,
        platform,
      ),
      size: cache.size_in_bytes,
      artifact: `rust-cache-${digest(`${cache.key}\0${cache.version}`)}`,
    };
    validateEntry(entry);
    if (!existing.has(`${entry.key}\0${entry.destinationVersion}`))
      entries.set(entry.artifact, entry);
  }
  return {
    schema: 1,
    sourceRepository: SOURCE,
    sourceRun: String(runId),
    sourceSha: sha,
    entries: [...entries.values()],
  };
}

export class CacheService {
  constructor({ upload = uploadBlocks } = {}) {
    check(process.env.ACTIONS_CACHE_SERVICE_V2, "Cache service v2 is required");
    this.url = safeUrl(process.env.ACTIONS_RESULTS_URL);
    this.token = process.env.ACTIONS_RUNTIME_TOKEN;
    check(this.token, "Cache runtime token is unavailable");
    this.upload = upload;
  }

  async call(method, data) {
    const response = await request(
      new URL(`/twirp/${SERVICE}/${method}`, this.url),
      {
        method: "POST",
        headers: {
          Authorization: `Bearer ${this.token}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify(data),
      },
    );
    return response.json();
  }

  async lookup(key, version) {
    check(
      /^[A-Za-z0-9_-]{1,512}$/.test(key) && VERSION.test(version),
      "Invalid cache lookup",
    );
    check(
      !["none", "write-only"].includes(process.env.ACTIONS_CACHE_MODE),
      "Cache reads are disabled",
    );
    const result = await this.call("GetCacheEntryDownloadURL", {
      key,
      version,
      restoreKeys: [],
    });
    if (!result.ok) return null;
    check(
      wireField(result, "matchedKey") === key,
      "Cache lookup returned a prefix match",
    );
    return safeUrl(wireField(result, "signedDownloadUrl"));
  }

  async save(filename, key, version) {
    check(
      !["none", "read"].includes(process.env.ACTIONS_CACHE_MODE),
      "Cache writes are disabled",
    );
    if (await this.lookup(key, version)) return false;
    const result = await this.call("CreateCacheEntry", { key, version });
    if (!result.ok) {
      if (await this.lookup(key, version)) return false;
      throw new Error("Cache reservation failed");
    }
    await this.upload(safeUrl(wireField(result, "signedUploadUrl")), filename);
    const finalized = await this.call("FinalizeCacheEntryUpload", {
      key,
      version,
      sizeBytes: String(fs.statSync(filename).size),
    });
    check(
      finalized.ok && (await this.lookup(key, version)),
      "Imported cache was not readable",
    );
    return true;
  }
}

async function uploadBlocks(url, filename) {
  const file = await fsp.open(filename, "r");
  const blocks = [];
  try {
    const size = (await file.stat()).size;
    for (let offset = 0; offset < size; offset += 64 * 1024 ** 2) {
      const chunk = Buffer.alloc(Math.min(64 * 1024 ** 2, size - offset));
      const { bytesRead } = await file.read(chunk, 0, chunk.length, offset);
      check(bytesRead === chunk.length, "Incomplete cache archive read");
      const block = Buffer.from(
        String(blocks.length).padStart(8, "0"),
      ).toString("base64");
      const blockUrl = new URL(url);
      blockUrl.searchParams.set("comp", "block");
      blockUrl.searchParams.set("blockid", block);
      await request(blockUrl, {
        method: "PUT",
        headers: { "x-ms-version": "2023-11-03" },
        body: chunk,
      });
      blocks.push(block);
    }
    const commitUrl = new URL(url);
    commitUrl.searchParams.set("comp", "blocklist");
    const body = `<?xml version="1.0" encoding="utf-8"?><BlockList>${blocks.map((block) => `<Latest>${block}</Latest>`).join("")}</BlockList>`;
    await request(commitUrl, {
      method: "PUT",
      headers: {
        "x-ms-version": "2023-11-03",
        "Content-Type": "application/xml",
      },
      body,
    });
  } finally {
    await file.close();
  }
}

export async function downloadArchive(
  service,
  key,
  version,
  expectedSize,
  filename,
) {
  const url = await service.lookup(key, version);
  check(url, "Source cache was evicted before export");
  const response = await request(url);
  await fsp.mkdir(path.dirname(filename), { recursive: true });
  const hash = crypto.createHash("sha256");
  let size = 0;
  await pipeline(
    Readable.fromWeb(response.body),
    new Transform({
      transform(chunk, encoding, callback) {
        size += chunk.length;
        if (size > expectedSize)
          return callback(new Error("Oversized cache download"));
        hash.update(chunk);
        callback(null, chunk);
      },
    }),
    fs.createWriteStream(filename, { flags: "wx" }),
  );
  check(size === expectedSize, "Incomplete cache download");
  return hash.digest("hex");
}

export async function downloadCache(service, entry, directory) {
  validateEntry(entry);
  const filename = path.join(directory, "cache.archive");
  const sha256 = await downloadArchive(
    service,
    entry.key,
    entry.version,
    entry.size,
    filename,
  );
  writeJson(path.join(directory, "manifest.json"), {
    ...entry,
    sha256,
  });
  return filename;
}

export async function verifyArchive(entry, directory) {
  const manifest = JSON.parse(
    await fsp.readFile(path.join(directory, "manifest.json"), "utf8"),
  );
  const { sha256, ...identity } = manifest;
  assert.deepEqual(identity, entry, "Cache artifact does not match its index");
  check(VERSION.test(sha256), "Invalid cache archive digest");
  const filename = path.join(directory, "cache.archive");
  check(
    fs.statSync(filename).size === entry.size,
    "Cache archive size mismatch",
  );
  const hash = crypto.createHash("sha256");
  for await (const chunk of fs.createReadStream(filename)) hash.update(chunk);
  check(hash.digest("hex") === sha256, "Cache archive digest mismatch");
  const validation = spawnSync(
    "python3",
    [
      new URL("validate-archive.py", import.meta.url).pathname,
      filename,
      entry.compression,
      JSON.stringify(validateEntry(entry).relativePaths),
    ],
    { encoding: "utf8" },
  );
  check(
    validation.status === 0,
    `Cache archive paths are not portable: ${validation.stderr.trim()}`,
  );
  return filename;
}

function requireContext(repository) {
  check(
    process.env.GITHUB_REPOSITORY === repository &&
      process.env.GITHUB_REF === REF,
    "Unexpected workflow context",
  );
  check(
    ["push", "workflow_dispatch"].includes(process.env.GITHUB_EVENT_NAME),
    "Unexpected workflow event",
  );
}

function validateIndex(index, run, sha) {
  check(
    index.schema === 1 &&
      index.sourceRepository === SOURCE &&
      index.sourceRun === String(run) &&
      index.sourceSha === sha &&
      Array.isArray(index.entries) &&
      index.entries.length <= 100,
    "Invalid cache index",
  );
  for (const entry of index.entries) {
    check(
      entry.sourceRun === String(run) && entry.sourceSha === sha,
      "Cache entry source mismatch",
    );
    validateEntry(entry);
  }
}

export async function findSource(github, sha, requested) {
  check(SHA.test(sha), "Invalid source SHA");
  const runs = requested
    ? [await github.get(`repos/${SOURCE}/actions/runs/${requested}`)]
    : await github.pages(
        `repos/${SOURCE}/actions/workflows/ci.yml/runs?event=push&branch=main&head_sha=${sha}&status=success`,
        "workflow_runs",
      );
  for (const run of runs) {
    validateSourceRun(run, sha);
    const artifacts = await github.pages(
      `repos/${SOURCE}/actions/runs/${run.id}/artifacts`,
      "artifacts",
    );
    if (
      artifacts.some(
        (artifact) => artifact.name === "rust-cache-index" && !artifact.expired,
      )
    )
      return String(run.id);
  }
  return "";
}

async function main() {
  const mode = input("mode");
  const directory = input("directory");
  const github = new GitHub();
  const run = input("source-run");
  const sha = input("source-sha") || process.env.GITHUB_SHA;
  if (mode === "plan-export") {
    requireContext(SOURCE);
    check(
      process.env.GITHUB_EVENT_NAME === "push",
      "Only push CI exports caches",
    );
    const index = await buildExportIndex(
      github,
      process.env.GITHUB_RUN_ID,
      process.env.GITHUB_SHA,
    );
    writeJson(path.join(directory, "index.json"), index);
    output("matrix", JSON.stringify({ include: index.entries }));
    output("has-entries", String(index.entries.length !== 0));
    console.log(`Exporting ${index.entries.length} missing Rust caches`);
  } else if (mode === "export") {
    requireContext(SOURCE);
    const entry = JSON.parse(input("entry"));
    check(
      entry.sourceRun === process.env.GITHUB_RUN_ID &&
        entry.sourceSha === process.env.GITHUB_SHA,
      "Unexpected export identity",
    );
    await downloadCache(new CacheService(), entry, directory);
  } else if (mode === "find") {
    requireContext(DESTINATION);
    if (run) check(/^\d+$/.test(run), "Invalid source run");
    output("run-id", await findSource(github, sha, run));
  } else if (mode === "plan-import") {
    requireContext(DESTINATION);
    validateSourceRun(
      await github.get(`repos/${SOURCE}/actions/runs/${run}`),
      sha,
    );
    const index = JSON.parse(
      await fsp.readFile(path.join(directory, "index.json"), "utf8"),
    );
    validateIndex(index, run, sha);
    const existing = new Set(
      (await github.caches(DESTINATION)).map(
        (entry) => `${entry.key}\0${entry.version}`,
      ),
    );
    const entries = index.entries.filter(
      (entry) => !existing.has(`${entry.key}\0${entry.destinationVersion}`),
    );
    output("matrix", JSON.stringify({ include: entries }));
    output("has-entries", String(entries.length !== 0));
  } else if (mode === "import") {
    requireContext(DESTINATION);
    validateSourceRun(
      await github.get(`repos/${SOURCE}/actions/runs/${run}`),
      sha,
    );
    const entry = JSON.parse(input("entry"));
    check(
      entry.sourceRun === run && entry.sourceSha === sha,
      "Unexpected import identity",
    );
    validateEntry(entry);
    const filename = await verifyArchive(entry, directory);
    const saved = await new CacheService().save(
      filename,
      entry.key,
      entry.destinationVersion,
    );
    console.log(
      saved ? `Imported ${entry.key}` : `Already cached: ${entry.key}`,
    );
  } else {
    throw new Error("Unknown Rust cache artifact mode");
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  main().catch((error) => {
    console.error(`Rust cache transfer failed: ${error.message}`);
    process.exitCode = 1;
  });
}
