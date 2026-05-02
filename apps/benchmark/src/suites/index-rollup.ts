import fs from "node:fs";
import path from "node:path";
import { log } from "../util/log.js";

const SUITES = [
  { name: "velocity", domain: "1.1 input velocity" },
  { name: "mutation-mix", domain: "1.2 mutation overhead" },
  { name: "subquery-depth", domain: "1.3 pipeline depth (reframed: SurQL subquery depth)" },
  { name: "join-fanout", domain: "2.1 join fan-out (write amplification)" },
  { name: "join-saturation", domain: "2.2 join state saturation" },
  { name: "view-density", domain: "3.2 view density (multi-tenant scaling)" },
  { name: "backlog", domain: "4.1 backlog recovery" },
];

const NOT_MEASURABLE = [
  {
    domain: "3.1 memory-to-disk",
    reason:
      "DBSP-on-SSP is in-memory only. There is no spill-to-disk for circuit state, so the 'performance cliff' this test would expose can't appear under the current architecture. Once a spill mechanism lands, this becomes measurable.",
  },
  {
    domain: "4.2 schema evolution",
    reason:
      "Field schemas are frozen at SSP bootstrap (apps/ssp/src/lib.rs:671-802). A `DEFINE FIELD` issued after the SSP starts won't be detected by the running circuit. Workaround: restart the SSP, covered by the backlog suite. Hot schema evolution requires changes to SSP bootstrap to support delta-on-schema.",
  },
];

export interface IndexArgs {
  resultsRoot: string;
}

export async function runIndex(args: IndexArgs): Promise<void> {
  const root = path.resolve(args.resultsRoot);
  if (!fs.existsSync(root)) {
    fs.mkdirSync(root, { recursive: true });
  }

  const lines: string[] = [];
  lines.push(`# sp00ky benchmarks, Performance Domains`);
  lines.push("");
  lines.push(
    `Cross-suite rollup. Each domain has its own \`summary.md\` linked below; this index gives a one-screen overview plus the two domains that aren't measurable on the current architecture.`,
  );
  lines.push("");
  lines.push(`Generated: ${new Date().toISOString()}`);
  lines.push("");

  lines.push(`## Suites`);
  lines.push("");
  lines.push(`| domain | suite | report | first 2 lines of headline |`);
  lines.push(`| --- | --- | --- | --- |`);
  for (const { name, domain } of SUITES) {
    const dir = path.join(root, name);
    const summaryPath = path.join(dir, "summary.md");
    if (!fs.existsSync(summaryPath)) {
      lines.push(`| ${domain} | \`${name}\` | _not yet run_ | n/a |`);
      continue;
    }
    const headline = extractHeadline(summaryPath);
    const rel = path.relative(root, summaryPath);
    lines.push(`| ${domain} | \`${name}\` | [${rel}](${rel}) | ${headline} |`);
  }
  lines.push("");

  lines.push(`## Not measurable on current sp00ky`);
  lines.push("");
  for (const { domain, reason } of NOT_MEASURABLE) {
    lines.push(`### ${domain}`);
    lines.push("");
    lines.push(reason);
    lines.push("");
  }

  lines.push(`## How to (re-)generate`);
  lines.push("");
  lines.push("```bash");
  lines.push(`for s in velocity mutation-mix subquery-depth join-fanout join-saturation view-density backlog; do`);
  lines.push(`  node apps/benchmark/dist/index.js $s --smoke || break`);
  lines.push(`done`);
  lines.push(`node apps/benchmark/dist/index.js index`);
  lines.push("```");
  lines.push("");

  const out = path.join(root, "index.md");
  fs.writeFileSync(out, lines.join("\n"));
  log.info(`Index written to ${out}`);
}

/** Pulls the first non-empty paragraph after the H1 from a suite summary. */
function extractHeadline(summaryPath: string): string {
  try {
    const md = fs.readFileSync(summaryPath, "utf8");
    const lines = md.split("\n");
    let pastTitle = false;
    let captured: string[] = [];
    for (const raw of lines) {
      const l = raw.trim();
      if (!pastTitle) {
        if (l.startsWith("# ")) pastTitle = true;
        continue;
      }
      if (l.length === 0) {
        if (captured.length > 0) break;
        continue;
      }
      if (l.startsWith("|") || l.startsWith("##")) break;
      captured.push(l);
      if (captured.join(" ").length > 200) break;
    }
    const text = captured.join(" ").replace(/\|/g, "\\|");
    return text.length > 220 ? text.slice(0, 220) + "…" : text || "n/a";
  } catch {
    return "n/a";
  }
}
