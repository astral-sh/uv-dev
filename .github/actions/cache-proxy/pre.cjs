const fs = require("node:fs");
const path = require("node:path");
const dns = require("node:dns").promises;
const { execFileSync } = require("node:child_process");
const {
  serviceUrl,
  request,
  readMetadata,
  emit,
  prefix,
} = require("./common.cjs");

async function main() {
  if (
    !["linux", "darwin"].includes(process.platform) ||
    process.env.GITHUB_REPOSITORY !== "astral-sh/uv-dev"
  )
    throw new Error(
      "This disposable prototype is restricted to uv-dev Linux/macOS jobs",
    );
  const key = process.env["INPUT_SEED-KEY"];
  if (key !== `${prefix()}-raw-seed`)
    throw new Error("Unexpected disposable seed key");
  const origins = {};
  for (const name of [
    "ACTIONS_CACHE_URL",
    "ACTIONS_RESULTS_URL",
    "ACTIONS_RUNTIME_URL",
  ]) {
    if (!process.env[name]) continue;
    const hostname = serviceUrl(process.env[name]).hostname;
    if (origins[hostname]) continue;
    const resolved = await Promise.allSettled([
      dns.resolve4(hostname),
      dns.resolve6(hostname),
    ]);
    const addresses = resolved.flatMap((result) =>
      result.status === "fulfilled" ? result.value : [],
    );
    if (!addresses.length) throw new Error("Could not resolve service origin");
    origins[hostname] = { addresses };
  }
  const hostname = serviceUrl(process.env.ACTIONS_RESULTS_URL).hostname;
  const address = origins[hostname].addresses.find(
    (value) => !value.includes(":"),
  );
  const baseline = await readMetadata(key, { address });
  if (baseline.status !== 200 || baseline.data.ok !== true)
    throw new Error("Same-run cache baseline was not readable");
  const plan = path.join(
    process.env.RUNNER_TEMP,
    "uv-cache-proxy-origins.json",
  );
  fs.writeFileSync(plan, JSON.stringify(origins));
  const macos = process.platform === "darwin";
  const directory = macos ? "/var/run/uv-cache-proxy" : "/run/uv-cache-proxy";
  const installer = macos ? "install-macos.py" : "install.py";
  execFileSync(
    "sudo",
    ["python3", path.join(__dirname, installer), "install", plan],
    { stdio: "inherit" },
  );
  const cert = macos
    ? `${directory}/ca.crt`
    : "/usr/local/share/ca-certificates/uv-cache-proxy.crt";
  const health = serviceUrl(process.env.ACTIONS_RESULTS_URL);
  health.pathname = "/__uv_cache_proxy_health";
  health.search = "";
  let healthy = false;
  for (let attempt = 0; attempt < 20; attempt++) {
    try {
      healthy =
        (await request(health, { ca: fs.readFileSync(cert) })).status === 200;
      if (healthy) break;
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  if (!healthy) throw new Error("Cache proxy failed its TLS health check");
  const bypass = [
    process.env.no_proxy || process.env.NO_PROXY || "",
    ...Object.keys(origins),
  ]
    .filter(Boolean)
    .join(",");
  const environment = {
    NODE_EXTRA_CA_CERTS: cert,
    UV_CACHE_PROXY_ACTIVE: "1",
    UV_CACHE_PROXY_CONFIG: `${directory}/origins.json`,
    no_proxy: bypass,
    NO_PROXY: bypass,
  };
  for (const [name, value] of Object.entries(environment)) {
    if (/[\r\n]/.test(value)) throw new Error("Unsafe environment value");
    fs.appendFileSync(process.env.GITHUB_ENV, `${name}=${value}\n`);
  }
  emit("bootstrap", {
    baselineReadable: true,
    proxyHealthy: healthy,
    serviceHostCount: Object.keys(origins).length,
    blockedAddressCount: new Set(
      Object.values(origins).flatMap((value) => value.addresses),
    ).size,
  });
}
main().catch((error) => {
  console.error(`::error::${error.message}`);
  process.exitCode = 1;
});
