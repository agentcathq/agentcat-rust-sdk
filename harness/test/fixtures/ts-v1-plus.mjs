// Test fixture: the TS v1 reference server plus a misbehavior chosen by env,
// to check that the driver sees requests a real server should not make.
//   FIXTURE_EXTRA_REQUEST=1    POST /v1/public/events (no trailing slash) at startup
//   FIXTURE_LATE_PUBLISH=1     one more (valid) publish after the client goes away
//                              (stdin close for stdio, SIGTERM for HTTP), then exit
//   FIXTURE_INVALID_PUBLISH=1  one publish at startup that breaks the publish contract
//                              (no project_id, session_id a number)
// Every request goes to the local mock (AGENTCAT_API_URL); the guard is loaded first.
import "../../src/egress-guard.mjs";

const api = process.env.AGENTCAT_API_URL;
const post = (path, body) =>
  fetch(`${api}${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });

if (process.env.FIXTURE_EXTRA_REQUEST === "1") {
  await post("/v1/public/events", { event_type: "fixture:extra" });
}

if (process.env.FIXTURE_INVALID_PUBLISH === "1") {
  await post("/v1/public/events/", {
    id: "evt_fixture_invalid",
    event_type: "mcp:tools/call",
    session_id: 5,
    timestamp: new Date().toISOString(),
  });
}

if (process.env.FIXTURE_LATE_PUBLISH === "1") {
  let sent = false;
  const late = () => {
    if (sent) return;
    sent = true;
    post("/v1/public/events/", {
      id: "evt_fixture_late",
      project_id: process.env.HARNESS_PROJECT_ID,
      event_type: "agentcat:custom",
      resource_name: "fixture:late",
      timestamp: new Date().toISOString(),
    }).finally(() => process.exit(0));
  };
  if (process.argv[2] === "stdio") process.stdin.on("end", late);
  // process.on (not once) keeps a SIGTERM listener installed, so the TS SDK's
  // own handler does not re-raise the signal before the late publish lands.
  else process.on("SIGTERM", late);
}

await import("../../src/servers/ts-v1.mjs");
