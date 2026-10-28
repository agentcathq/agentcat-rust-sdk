// Vector generator: runs the REAL TS modules (pinned checkout) over a corpus
// of wire-shaped inputs and writes expected outputs as JSON. Rust tests read
// these files; nobody retypes expected values.
import { writeFileSync, readFileSync } from "node:fs";
import { estimateInputTokens, estimateOutputTokens, utf8ByteLength } from "../ts-sdk-src/src/modules/tokenEstimate.ts";
import { deriveSessionId, isValidSessionId } from "../ts-sdk-src/src/modules/handles.ts";
import { normalize } from "../ts-sdk-src/src/modules/truncation.ts";

const sha = process.env.TS_SHA ?? "unknown";
// Corpus: JSON TEXT as it arrives on the wire (parsed exactly as the SDK would).
const argsCorpus: string[] = [
  `{"msg":"hi"}`, `{}`, `null`, `{"n":1.0}`, `{"n":1e21}`, `{"n":-0}`, `{"n":0.1}`,
  `{"n":123456789012345678901234567890}`, `{"s":"\\u00e9\\u4e2d\\ud83d\\ude00"}`,
  `{"s":"\\ud800 lone high surrogate"}`, `{"b":true,"z":null,"a":[1,"2",{"x":[]}]}`,
  `{"emoji":"${"😀".repeat(50)}"}`, `{"k\\u0000":"nul"}`,
];
const tokenVectors = argsCorpus.map((text) => {
  const value = JSON.parse(text);
  return { input_json: text, input_tokens: estimateInputTokens(value) ?? null, output_tokens: estimateOutputTokens(value) ?? null };
});
const deriveVectors = [["corr-7", "proj_DIFFSPIKE000000000000000000"], ["corr-7", undefined], ["", "proj_x"], ["a:b", "c"], ["a", "b:c"], ["é中😀", "proj_y"]]
  .map(([id, projectId]) => ({ id, project_id: projectId ?? null, session_id: deriveSessionId(id as string, projectId as string | undefined) }));
const truncationCorpus = [ "x".repeat(40000), "😀".repeat(20000), "a" + "é".repeat(33000) ];
const truncationVectors = truncationCorpus.map((s) => {
  const out = normalize(s) as string;
  return { input_utf16_len: s.length, input_head: s.slice(0, 8), output_utf16_len: out.length, output_utf8_len: utf8ByteLength(out), output_tail: out.slice(-24) };
});
writeFileSync(new URL("./vectors.json", import.meta.url), JSON.stringify({ ts_sha: sha, generator: "gen-vectors.ts", token_estimate: tokenVectors, derive_session_id: deriveVectors, truncation_string: truncationVectors }, null, 2));
console.log(`wrote ${tokenVectors.length} token, ${deriveVectors.length} derive, ${truncationVectors.length} truncation vectors @ ${sha}`);
