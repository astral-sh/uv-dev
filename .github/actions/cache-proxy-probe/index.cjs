const fs = require("node:fs");
const path = require("node:path");
const {
  content,
  version,
  prefix,
  output,
  serviceUrl,
  request,
  cacheCall,
  readMetadata,
  isDenied,
  storage,
  writeCache,
  emit,
} = require("../cache-proxy/common.cjs");

async function main() {
  const operation = process.env.INPUT_OPERATION;
  const seedKey = `${prefix()}-raw-seed`;
  if (operation === "seed") {
    const write = await writeCache(seedKey);
    const metadata = await readMetadata(seedKey);
    const url =
      metadata.data.signed_download_url || metadata.data.signedDownloadUrl;
    const downloaded = url ? await storage(url) : null;
    const matches =
      downloaded?.ok &&
      Buffer.from(await downloaded.arrayBuffer()).equals(content);
    emit("seed", { seedKey, version, write, readable: !!matches });
    if (!write.finalized || !matches)
      throw new Error("Harmless seed cache roundtrip failed");
    output("seed-key", seedKey);
    output("version", version);
    fs.mkdirSync("cache-proxy-marker", { recursive: true });
    fs.writeFileSync("cache-proxy-marker/marker.txt", content);
    return;
  }
  if (operation !== "probe") throw new Error("Unknown probe operation");
  const read = await readMetadata(seedKey);
  const write = await writeCache(`${prefix()}-raw-blocked`);
  const legacy = serviceUrl(
    process.env.ACTIONS_CACHE_URL || process.env.ACTIONS_RUNTIME_URL,
  );
  legacy.pathname =
    legacy.pathname.replace(/\/?$/, "/") + "_apis/artifactcache/cache";
  legacy.search = new URLSearchParams({ keys: seedKey, version }).toString();
  const legacyResult = await request(legacy, {
    headers: { Authorization: `Bearer ${process.env.ACTIONS_RUNTIME_TOKEN}` },
  });
  const origins = JSON.parse(
    fs.readFileSync(process.env.UV_CACHE_PROXY_CONFIG, "utf8"),
  );
  const hostname = serviceUrl(process.env.ACTIONS_RESULTS_URL).hostname;
  const direct = [];
  for (const address of origins[hostname].addresses) {
    try {
      const response = await readMetadata(seedKey, { address, timeout: 4000 });
      direct.push({
        family: address.includes(":") ? 6 : 4,
        http: response.status,
        blocked: false,
      });
    } catch (error) {
      direct.push({
        family: address.includes(":") ? 6 : 4,
        error: error.code,
        blocked: [
          "ECONNRESET",
          "ECONNREFUSED",
          "ETIMEDOUT",
          "ENETUNREACH",
          "EHOSTUNREACH",
        ].includes(error.code),
      });
    }
  }
  const oidc = serviceUrl(process.env.ACTIONS_ID_TOKEN_REQUEST_URL);
  const audience = "urn:uv-cache-proxy-probe";
  oidc.searchParams.set("audience", audience);
  const response = await request(oidc, {
    headers: {
      Authorization: `Bearer ${process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN}`,
    },
  });
  let oidcWorks = false;
  try {
    const token = JSON.parse(response.body).value;
    const claims = JSON.parse(Buffer.from(token.split(".")[1], "base64url"));
    oidcWorks =
      response.status === 200 &&
      claims.aud === audience &&
      claims.repository === "astral-sh/uv-dev" &&
      String(claims.run_id) === process.env.GITHUB_RUN_ID;
  } catch {}
  const artifactMatches = fs
    .readFileSync(
      path.join(process.env.RUNNER_TEMP, "seed-artifact", "marker.txt"),
    )
    .equals(content);
  const result = {
    rawReadDenied: isDenied(read, "read"),
    rawWrite: write,
    legacyReadDenied:
      legacyResult.status === 403 &&
      legacyResult.headers["x-uv-cache-proxy"] === "denied",
    direct,
    oidcWorks,
    artifactMatches,
    audit: JSON.parse(
      fs.readFileSync("/run/uv-cache-proxy/audit.json", "utf8"),
    ),
  };
  emit("protected-job", result);
  fs.writeFileSync(
    path.join(process.env.RUNNER_TEMP, "cache-proxy-evidence.json"),
    JSON.stringify(result, null, 2) + "\n",
  );
  if (
    !result.rawReadDenied ||
    !write.denied ||
    !result.legacyReadDenied ||
    !direct.length ||
    !direct.every((value) => value.blocked) ||
    !oidcWorks ||
    !artifactMatches
  )
    throw new Error("Cache proxy acceptance check failed");
}
main().catch((error) => {
  console.error(`::error::${error.message}`);
  process.exitCode = 1;
});
