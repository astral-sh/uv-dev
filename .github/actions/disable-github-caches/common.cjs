const fs = require("node:fs");
const http = require("node:http");
const https = require("node:https");
const net = require("node:net");
const path = require("node:path");

function enabled() {
  const value = process.env.INPUT_ENABLED || "true";
  if (!["true", "false"].includes(value))
    throw new Error("Invalid enabled input");
  return value === "true";
}

function platform() {
  const windows = process.platform === "win32";
  const macos = process.platform === "darwin";
  if (!windows && !macos && process.platform !== "linux")
    throw new Error("Unsupported runner platform");
  const directory = windows
    ? path.join(process.env.ProgramData, "uv-cache-proxy")
    : macos
      ? "/var/run/uv-cache-proxy"
      : "/run/uv-cache-proxy";
  return {
    windows,
    directory,
    command: windows ? "python" : "sudo",
    arguments: windows ? [] : ["python3"],
    installer: path.join(
      __dirname,
      windows
        ? "install-windows.py"
        : macos
          ? "install-macos.py"
          : "install.py",
    ),
    certificate:
      windows || macos
        ? path.join(directory, "ca.crt")
        : "/usr/local/share/ca-certificates/uv-cache-proxy.crt",
  };
}

function serviceUrl(value) {
  const url = new URL(value);
  const depot =
    url.protocol === "http:" &&
    net.isIPv4(url.hostname) &&
    url.hostname.startsWith("10.") &&
    ["977", "978"].includes(url.port);
  const github =
    url.protocol === "https:" &&
    url.hostname.endsWith(".actions.githubusercontent.com") &&
    !url.port;
  if ((!depot && !github) || url.username || url.password)
    throw new Error("Unexpected cache service endpoint");
  return url;
}

function request(
  url,
  { method = "GET", headers = {}, body, address, ca, timeout = 10000 } = {},
) {
  return new Promise((resolve, reject) => {
    const options = { method, headers, agent: false, ...(ca ? { ca } : {}) };
    if (address)
      options.lookup = (_host, lookupOptions, callback) => {
        const family = address.includes(":") ? 6 : 4;
        if (lookupOptions.all) callback(null, [{ address, family }]);
        else callback(null, address, family);
      };
    const transport = url.protocol === "http:" ? http : https;
    const outgoing = transport.request(url, options, (response) => {
      const chunks = [];
      let length = 0;
      response.on("data", (chunk) => {
        length += chunk.length;
        if (length > 1024 * 1024)
          outgoing.destroy(
            Object.assign(new Error("Response too large"), { code: "ETOOBIG" }),
          );
        else chunks.push(chunk);
      });
      response.on("end", () =>
        resolve({
          status: response.statusCode,
          headers: response.headers,
          body: Buffer.concat(chunks),
        }),
      );
    });
    outgoing.setTimeout(timeout, () =>
      outgoing.destroy(
        Object.assign(new Error("Request timeout"), { code: "ETIMEDOUT" }),
      ),
    );
    outgoing.on("error", (error) =>
      reject(
        Object.assign(new Error("Cache service request failed"), {
          code: error.code || "REQUEST_FAILED",
        }),
      ),
    );
    if (body) outgoing.write(body);
    outgoing.end();
  });
}

function exportVariable(name, value) {
  if (/[\r\n]/.test(value)) throw new Error("Unsafe environment value");
  fs.appendFileSync(process.env.GITHUB_ENV, `${name}=${value}\n`);
}

module.exports = { enabled, platform, serviceUrl, request, exportVariable };
