const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { enabled, platform } = require("./common.cjs");
try {
  if (enabled() && process.env.STATE_installed === "true") {
    const runner = platform();
    const audit = JSON.parse(
      fs.readFileSync(path.join(runner.directory, "audit.json"), "utf8"),
    );
    execFileSync(
      runner.command,
      [...runner.arguments, runner.installer, "cleanup"],
      { stdio: "inherit" },
    );
    console.log(
      `Cache proxy removed. Denied ${audit.cache_read_denied || 0} read requests and ${audit.cache_write_denied || 0} write requests.`,
    );
  }
} catch (error) {
  console.error(`::error::${error.message}`);
  process.exitCode = 1;
}
