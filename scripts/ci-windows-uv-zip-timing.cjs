// Observe setup-uv's normal HTTP, file and tar operations without consuming its response stream.
const fs = require("node:fs");
const http = require("node:http");
const https = require("node:https");
const path = require("node:path");
const childProcess = require("node:child_process");
const { monitorEventLoopDelay, performance } = require("node:perf_hooks");

const arm = process.env.ZIP_SOURCE;
const root = process.env.RUNNER_TEMP;
// Give each action invocation an empty tool cache, even when it runs second.
process.env.RUNNER_TOOL_CACHE = path.join(root, "uv-zip-tool-cache-" + arm);
fs.mkdirSync(process.env.RUNNER_TOOL_CACHE, { recursive: true });
const redirectTargets = new Set();
const downloadedFiles = new Set();
const events = [];
let requests = 0;
const loopDelay = monitorEventLoopDelay({ resolution: 20 });

function emit(event, data = {}) {
    const record = { event, ms: Number(performance.now().toFixed(2)), ...data };
    events.push(record);
    console.log("ZIP_NODE " + JSON.stringify({ arm, ...record }));
}

for (const transport of [http, https]) {
    const originalRequest = transport.request;
    transport.request = function (...args) {
        const request = originalRequest.apply(this, args);
        const host = String(request.getHeader("host") || request.host);
        const key = host + request.path;
        const isZip = request.path.split("?")[0].endsWith("/uv-x86_64-pc-windows-msvc.zip");
        if (!isZip && !redirectTargets.has(key)) return request;
        const id = ++requests;
        if (id === 1) loopDelay.enable();
        emit("request", { id, host, utc_ms: Date.now() });
        request.once("socket", (socket) => {
            emit("socket", { id, reused: request.reusedSocket, connecting: socket.connecting });
            if (socket.connecting) {
                socket.once("lookup", () => emit("dns", { id }));
                socket.once("connect", () => emit("tcp", { id }));
                socket.once("secureConnect", () => emit("tls", { id }));
            }
        });
        request.once("finish", () => emit("sent", { id }));
        request.once("error", (error) => emit("request_error", { id, code: error.code }));
        request.prependOnceListener("response", (response) => {
            emit("headers", { id, status: response.statusCode, size: response.headers["content-length"] });
            if (response.headers.location) {
                const next = new URL(response.headers.location, "https://" + key);
                redirectTargets.add(next.host + next.pathname + next.search);
            }
            let bytes = 0;
            let chunks = 0;
            let lastData = performance.now();
            let largestGap = 0;
            let gapAt = 0;
            const originalPush = response.push;
            response.push = function (chunk, ...rest) {
                if (chunk !== null) {
                    const now = performance.now();
                    if (bytes === 0) emit("first_body", { id });
                    chunks++;
                    if (now - lastData > largestGap) {
                        largestGap = now - lastData;
                        gapAt = bytes;
                    }
                    bytes += chunk.length;
                    lastData = now;
                } else {
                    emit("body_end", { id, bytes, chunks, max_gap_ms: Math.round(largestGap),
                        gap_at_bytes: gapAt, last_payload_to_eof_ms: Math.round(performance.now() - lastData),
                        max_event_loop_delay_ms: Math.round(loopDelay.max / 1e6) });
                }
                return originalPush.call(this, chunk, ...rest);
            };
            response.once("aborted", () => emit("aborted", { id, bytes }));
        });
        return request;
    };
}

const writeStream = fs.createWriteStream;
fs.createWriteStream = function (file, ...args) {
    const stream = writeStream.call(this, file, ...args);
    if (!requests || typeof file !== "string") return stream;
    downloadedFiles.add(path.resolve(file));
    emit("write_start");
    let blocked = null;
    let blockedMs = 0;
    let maxBlockedMs = 0;
    function recordDrain() {
        if (blocked === null) return;
        const elapsed = performance.now() - blocked;
        blockedMs += elapsed;
        maxBlockedMs = Math.max(maxBlockedMs, elapsed);
        blocked = null;
    }
    const write = stream.write;
    stream.write = function (...args) {
        const ready = write.apply(this, args);
        if (!ready && blocked === null) blocked = performance.now();
        return ready;
    };
    stream.on("drain", recordDrain);
    stream.once("finish", () => {
        recordDrain();
        emit("write_end", { bytes: stream.bytesWritten, backpressure_ms: Math.round(blockedMs),
            max_backpressure_ms: Math.round(maxBlockedMs) });
    });
    return stream;
};

const readStream = fs.createReadStream;
fs.createReadStream = function (file, ...args) {
    const stream = readStream.call(this, file, ...args);
    if (typeof file === "string" && downloadedFiles.has(path.resolve(file))) {
        emit("hash_start");
        stream.once("end", () => emit("hash_end", { bytes: stream.bytesRead }));
    }
    return stream;
};

const spawn = childProcess.spawn;
childProcess.spawn = function (command, ...args) {
    const tar = /^(tar|tar\.exe)$/i.test(path.basename(command)) && requests > 0;
    if (tar) emit("tar_start");
    const child = spawn.call(this, command, ...args);
    if (tar) child.once("close", (code) => {
        loopDelay.disable();
        emit("tar_end", { code, utc_ms: Date.now(), max_event_loop_delay_ms: Math.round(loopDelay.max / 1e6) });
    });
    return child;
};

process.once("exit", (code) => {
    if (!requests) return;
    const report = { arm, code, runner: process.env.RUNNER_NAME, node: process.version,
        time_origin: performance.timeOrigin, events };
    fs.writeFileSync(path.join(root, "uv-zip-node-" + arm + ".json"), JSON.stringify(report, null, 2));
});
