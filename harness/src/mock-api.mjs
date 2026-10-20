// Mock AgentCat API: records every inbound HTTP request verbatim.
//   POST /v1/public/events/   -> 202 {"status":"accepted"} (mirrors agentcat-server public.py)
//   GET  /__requests           -> JSON array of every recorded request (control plane)
//   POST /__reset              -> clears the recording
//   POST /__fail?n=K&status=S  -> the next K publish calls answer status S (retry tests)
// Anything else is recorded and answered 404, so unexpected calls show up in the diff.
//
// usage: node harness/src/mock-api.mjs [port]   (0 or none: an ephemeral port)
// Prints one ready line on stdout: {"ready": true, "port": <port>}. Binds 127.0.0.1 only.
import http from "node:http";

const port = Number(process.argv[2] ?? 0);
let requests = [];
let failBudget = 0;
let failStatus = 500;
let seq = 0;

const server = http.createServer((req, res) => {
  const chunks = [];
  req.on("data", (c) => chunks.push(c));
  req.on("end", () => {
    const url = new URL(req.url, "http://mock");
    if (url.pathname === "/__requests") {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify(requests));
      return;
    }
    if (url.pathname === "/__reset") {
      requests = [];
      seq = 0;
      res.writeHead(204).end();
      return;
    }
    if (url.pathname === "/__fail") {
      failBudget = Number(url.searchParams.get("n") ?? 1);
      failStatus = Number(url.searchParams.get("status") ?? 500);
      res.writeHead(204).end();
      return;
    }
    const raw = Buffer.concat(chunks).toString("utf8");
    let body = raw;
    try {
      body = raw ? JSON.parse(raw) : null;
    } catch {
      // keep the raw text: a non-JSON body is itself a finding
    }
    const isPublish = req.method === "POST" && url.pathname === "/v1/public/events/";
    let status = isPublish ? 202 : 404;
    if (isPublish && failBudget > 0) {
      failBudget -= 1;
      status = failStatus;
    }
    requests.push({
      seq: seq++,
      receivedAt: Date.now(),
      method: req.method,
      path: url.pathname,
      query: url.search,
      headers: {
        "content-type": req.headers["content-type"],
        "user-agent": req.headers["user-agent"],
        accept: req.headers["accept"],
      },
      status,
      body,
    });
    res.writeHead(status, { "content-type": "application/json" });
    res.end(JSON.stringify(status === 202 ? { status: "accepted" } : { error: "mock" }));
  });
});

server.listen(port, "127.0.0.1", () => {
  process.stdout.write(`${JSON.stringify({ ready: true, port: server.address().port })}\n`);
});
