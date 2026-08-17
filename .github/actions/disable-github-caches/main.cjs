const { enabled } = require("./common.cjs");
if (enabled() && process.env.UV_CACHE_PROXY_ACTIVE !== "1")
  throw new Error("Cache-denial pre hook did not complete");
