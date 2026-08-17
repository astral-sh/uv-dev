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
    !["linux", "darwin", "win32"].includes(process.platform) ||
    process.env.GITHUB_REPOSITORY !== "astral-sh/uv-dev"
  )
    throw new Error("This disposable prototype is restricted to uv-dev jobs");
  const key = process.env["INPUT_SEED-KEY"];
  if (key !== `${prefix()}-raw-seed`)
    throw new Error("Unexpected disposable seed key");
  const origins = {};
  const endpoints = [
    process.env.ACTIONS_CACHE_URL,
    process.env.ACTIONS_RESULTS_URL,
    process.env.ACTIONS_RUNTIME_URL,
    "https://artifactcache.actions.githubusercontent.com",
    "https://results-receiver.actions.githubusercontent.com",
  ];
  for (const endpoint of endpoints) {
    if (!endpoint) continue;
    const url = serviceUrl(endpoint);
    const hostname = url.hostname;
    if (url.protocol === "http:") {
      origins[url.host] = {
        scheme: "http",
        port: Number(url.port),
        listen_port: Number(url.port) + 19000,
        forward_origin:
          url.port === "978"
            ? "results-receiver.actions.githubusercontent.com"
            : "artifactcache.actions.githubusercontent.com",
        addresses: [hostname],
      };
      continue;
    }
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
  const resultUrl = serviceUrl(process.env.ACTIONS_RESULTS_URL);
  const address = origins[resultUrl.host].addresses.find(
    (value) => !value.includes(":"),
  );
  const baseline = await readMetadata(key, { address });
  const githubBaseline =
    resultUrl.protocol === "http:"
      ? await readMetadata(key, {
          base: "https://results-receiver.actions.githubusercontent.com",
          address: origins[
            "results-receiver.actions.githubusercontent.com"
          ].addresses.find((value) => !value.includes(":")),
        })
      : baseline;
  if (
    baseline.status !== 200 ||
    githubBaseline.status !== 200 ||
    githubBaseline.data.ok !== true
  )
    throw new Error("Same-run cache baseline was not readable");
  const plan = path.join(
    process.env.RUNNER_TEMP,
    "uv-cache-proxy-origins.json",
  );
  fs.writeFileSync(plan, JSON.stringify(origins));
  const macos = process.platform === "darwin";
  const windows = process.platform === "win32";
  const directory = windows
    ? path.join(process.env.ProgramData, "uv-cache-proxy")
    : macos
      ? "/var/run/uv-cache-proxy"
      : "/run/uv-cache-proxy";
  const installer = windows
    ? "install-windows.py"
    : macos
      ? "install-macos.py"
      : "install.py";
  execFileSync(
    windows ? "python" : "sudo",
    [
      ...(windows ? [] : ["python3"]),
      path.join(__dirname, installer),
      "install",
      plan,
    ],
    { stdio: "inherit" },
  );
  const cert =
    macos || windows
      ? path.join(directory, "ca.crt")
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
    directIsolation: !windows,
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
