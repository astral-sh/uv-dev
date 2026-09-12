const { spawnSync } = require("node:child_process");
const path = require("node:path");

module.exports = (phase) => {
  const result = spawnSync(
    "python3",
    [path.join(__dirname, "probe.py"), phase],
    {
      stdio: "inherit",
    },
  );
  if (result.error) console.error("::error::Could not start probe");
  process.exitCode = result.status ?? 1;
};
