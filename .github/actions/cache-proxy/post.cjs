const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { emit } = require("./common.cjs");
try {
  if (fs.existsSync("/run/uv-cache-proxy/audit.json"))
    emit(
      "final-audit",
      JSON.parse(fs.readFileSync("/run/uv-cache-proxy/audit.json", "utf8")),
    );
  execFileSync(
    "sudo",
    ["python3", path.join(__dirname, "install.py"), "cleanup"],
    { stdio: "inherit" },
  );
  console.log("Disposable cache proxy removed.");
} catch (error) {
  console.error(`::error::${error.message}`);
  process.exitCode = 1;
}
