const { spawnSync } = require("node:child_process");
const path = require("node:path");

module.exports = (operation) => {
  const result = spawnSync(
    "python3",
    [path.join(__dirname, "action.py"), operation],
    {
      stdio: "inherit",
    },
  );
  if (result.error) console.error("::error::Could not start Python");
  process.exitCode = result.status ?? 1;
};
