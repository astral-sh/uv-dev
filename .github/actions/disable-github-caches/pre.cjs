const fs = require("node:fs");
const path = require("node:path");
const dns = require("node:dns").promises;
const crypto = require("node:crypto");
const { execFileSync } = require("node:child_process");
const {
  enabled,
  platform,
  serviceUrl,
  request,
  exportVariable,
} = require("./common.cjs");

async function main() {
  if (!enabled()) return;
  const runner = platform();
  if (!process.env.ACTIONS_CACHE_URL || !process.env.ACTIONS_RESULTS_URL)
    throw new Error("Runner did not provide both cache service endpoints");
  const endpoints = [
    process.env.ACTIONS_CACHE_URL,
    process.env.ACTIONS_RESULTS_URL,
    "https://artifactcache.actions.githubusercontent.com",
    "https://results-receiver.actions.githubusercontent.com",
  ];
  const origins = {};
  for (const endpoint of endpoints) {
    const url = serviceUrl(endpoint);
    if (url.protocol === "http:") {
      if (process.platform !== "linux")
        throw new Error("Private cache endpoints require Linux");
      origins[url.host] = {
        scheme: "http",
        port: Number(url.port),
        listen_port: Number(url.port) + 19000,
        forward_origin:
          url.port === "978"
            ? "results-receiver.actions.githubusercontent.com"
            : "artifactcache.actions.githubusercontent.com",
        addresses: [url.hostname],
      };
      continue;
    }
    if (origins[url.hostname]) continue;
    const resolved = await Promise.allSettled([
      dns.resolve4(url.hostname),
      dns.resolve6(url.hostname),
    ]);
    const addresses = resolved.flatMap((result) =>
      result.status === "fulfilled" ? result.value : [],
    );
    if (!addresses.length)
      throw new Error("Could not resolve cache service origin");
    origins[url.hostname] = { addresses };
  }
  const plan = path.join(
    process.env.RUNNER_TEMP,
    "uv-release-cache-origins.json",
  );
  fs.writeFileSync(plan, JSON.stringify(origins));
  let installationStarted = false;
  try {
    installationStarted = true;
    execFileSync(
      runner.command,
      [...runner.arguments, runner.installer, "install", plan],
      { stdio: "inherit" },
    );
    const certificate = fs.readFileSync(runner.certificate);
    const health = serviceUrl(process.env.ACTIONS_RESULTS_URL);
    health.pathname = "/__uv_cache_proxy_health";
    health.search = "";
    let healthy = false;
    for (let attempt = 0; attempt < 20; attempt++) {
      try {
        healthy = (await request(health, { ca: certificate })).status === 200;
        if (healthy) break;
      } catch {}
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
    if (!healthy) throw new Error("Cache proxy failed its health check");
    const key = `uv-release-cache-check-${crypto.randomUUID()}`;
    const version = crypto.randomBytes(32).toString("hex");
    const body = Buffer.from(
      JSON.stringify({ key, version, restore_keys: [] }),
    );
    for (const endpoint of new Set([
      process.env.ACTIONS_RESULTS_URL,
      "https://results-receiver.actions.githubusercontent.com",
    ])) {
      const url = serviceUrl(endpoint);
      url.pathname =
        "/twirp/github.actions.results.api.v1.CacheService/GetCacheEntryDownloadURL";
      url.search = "";
      const response = await request(url, {
        method: "POST",
        ca: certificate,
        headers: {
          Authorization: `Bearer ${process.env.ACTIONS_RUNTIME_TOKEN}`,
          "Content-Type": "application/json",
          "Content-Length": String(body.length),
        },
        body,
      });
      if (
        response.status !== 403 ||
        response.headers["x-uv-cache-proxy"] !== "denied"
      )
        throw new Error("Cache v2 denial self-test failed");
    }
    const legacy = serviceUrl(process.env.ACTIONS_CACHE_URL);
    legacy.pathname =
      legacy.pathname.replace(/\/?$/, "/") + "_apis/artifactcache/cache";
    legacy.search = new URLSearchParams({ keys: key, version }).toString();
    const legacyResponse = await request(legacy, {
      ca: certificate,
      headers: { Authorization: `Bearer ${process.env.ACTIONS_RUNTIME_TOKEN}` },
    });
    if (
      legacyResponse.status !== 403 ||
      legacyResponse.headers["x-uv-cache-proxy"] !== "denied"
    )
      throw new Error("Legacy cache denial self-test failed");
    exportVariable("NODE_EXTRA_CA_CERTS", runner.certificate);
    exportVariable("UV_CACHE_PROXY_ACTIVE", "1");
    exportVariable(
      "UV_CACHE_PROXY_CONFIG",
      path.join(runner.directory, "origins.json"),
    );
    const bypass = [
      process.env.no_proxy || process.env.NO_PROXY || "",
      ...Object.keys(origins),
    ]
      .filter(Boolean)
      .join(",");
    exportVariable("no_proxy", bypass);
    exportVariable("NO_PROXY", bypass);
    fs.appendFileSync(process.env.GITHUB_STATE, "installed=true\n");
    console.log(
      `GitHub cache API denial is active (${runner.windows ? "DNS interception" : "DNS interception and direct-address filtering"}).`,
    );
  } catch (error) {
    if (installationStarted) {
      try {
        execFileSync(
          runner.command,
          [...runner.arguments, runner.installer, "cleanup"],
          { stdio: "inherit" },
        );
      } catch {
        console.error("::error::Cache proxy cleanup failed");
      }
    }
    throw error;
  }
}
main().catch((error) => {
  console.error(`::error::${error.message}`);
  process.exitCode = 1;
});
