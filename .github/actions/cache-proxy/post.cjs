const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { emit } = require("./common.cjs");
try {
  const windows = process.platform === "win32";
  const directory = windows
    ? path.join(process.env.ProgramData, "uv-cache-proxy")
    : process.platform === "darwin"
      ? "/var/run/uv-cache-proxy"
      : "/run/uv-cache-proxy";
  const installer = windows
    ? "install-windows.py"
    : process.platform === "darwin"
      ? "install-macos.py"
      : "install.py";
  if (fs.existsSync(`${directory}/audit.json`))
    emit(
      "final-audit",
      JSON.parse(fs.readFileSync(`${directory}/audit.json`, "utf8")),
    );
  execFileSync(
    windows ? "python" : "sudo",
    [
      ...(windows ? [] : ["python3"]),
      path.join(__dirname, installer),
      "cleanup",
    ],
    { stdio: "inherit" },
  );
  console.log("Disposable cache proxy removed.");
} catch (error) {
  console.error(`::error::${error.message}`);
  process.exitCode = 1;
}
