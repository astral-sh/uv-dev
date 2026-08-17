const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { emit } = require("./common.cjs");
try {
  const directory =
    process.platform === "darwin"
      ? "/var/run/uv-cache-proxy"
      : "/run/uv-cache-proxy";
  const installer =
    process.platform === "darwin" ? "install-macos.py" : "install.py";
  if (fs.existsSync(`${directory}/audit.json`))
    emit(
      "final-audit",
      JSON.parse(fs.readFileSync(`${directory}/audit.json`, "utf8")),
    );
  execFileSync(
    "sudo",
    ["python3", path.join(__dirname, installer), "cleanup"],
    { stdio: "inherit" },
  );
  console.log("Disposable cache proxy removed.");
} catch (error) {
  console.error(`::error::${error.message}`);
  process.exitCode = 1;
}
