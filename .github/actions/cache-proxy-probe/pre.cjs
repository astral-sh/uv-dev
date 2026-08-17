const {
  prefix,
  readMetadata,
  isDenied,
  emit,
} = require("../cache-proxy/common.cjs");
async function main() {
  if (process.env.INPUT_OPERATION !== "probe") return;
  const response = await readMetadata(`${prefix()}-raw-seed`);
  const denied =
    process.env.UV_CACHE_PROXY_ACTIVE === "1" && isDenied(response, "read");
  emit("later-action-pre", { denied });
  if (!denied) throw new Error("A later action pre hook could read cache");
}
main().catch((error) => {
  console.error(`::error::${error.message}`);
  process.exitCode = 1;
});
