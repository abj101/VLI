#!/usr/bin/env node
/**
 * Run composer live eval (ignored cargo tests) and print a pass/fail summary.
 *
 * Usage:
 *   node scripts/composer-eval.mjs [--report]
 *
 * --report  Write composer-eval-report.json under jarvis/ (timestamp + counts).
 */

import { spawnSync } from "child_process";
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const JARVIS_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const writeReport = process.argv.includes("--report");

const TEST_LINE = /^test\s+(\S+)\s+\.\.\.\s+(ok|FAILED|ignored)$/;
const RESULT_LINE =
  /test result: (ok|FAILED)\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored/;

/**
 * @param {string} output
 */
function parseCargoTestOutput(output) {
  /** @type {{ name: string; status: "ok" | "FAILED" | "ignored" }[]} */
  const tests = [];

  for (const line of output.split(/\r?\n/)) {
    const match = line.match(TEST_LINE);
    if (match) {
      tests.push({ name: match[1], status: /** @type {"ok"|"FAILED"|"ignored"} */ (match[2]) });
    }
  }

  const summaryMatch = output.match(RESULT_LINE);
  const summary = summaryMatch
    ? {
        result: summaryMatch[1],
        passed: Number(summaryMatch[2]),
        failed: Number(summaryMatch[3]),
        ignored: Number(summaryMatch[4]),
      }
    : {
        result: tests.some((t) => t.status === "FAILED") ? "FAILED" : "ok",
        passed: tests.filter((t) => t.status === "ok").length,
        failed: tests.filter((t) => t.status === "FAILED").length,
        ignored: tests.filter((t) => t.status === "ignored").length,
      };

  return { tests, summary, output };
}

/**
 * @param {{ name: string; status: string }[]} tests
 */
function printSummaryTable(tests) {
  if (tests.length === 0) {
    console.log("No individual test lines parsed (see cargo output above).");
    return;
  }

  const nameWidth = Math.max(4, ...tests.map((t) => t.name.length));
  const statusWidth = 6;
  const divider = `+-${"-".repeat(nameWidth)}-+-${"-".repeat(statusWidth)}-+`;

  console.log("\nComposer live eval summary\n");
  console.log(divider);
  console.log(`| ${"Test".padEnd(nameWidth)} | ${"Status".padEnd(statusWidth)} |`);
  console.log(divider);
  for (const { name, status } of tests) {
    console.log(`| ${name.padEnd(nameWidth)} | ${status.padEnd(statusWidth)} |`);
  }
  console.log(divider);
}

function runLiveEval() {
  const nodeCmd = process.execPath;
  const args = ["scripts/test-composer.mjs", "--live"];

  console.log(`composer-eval: ${nodeCmd} ${args.join(" ")}\n`);

  const child = spawnSync(nodeCmd, args, {
    cwd: JARVIS_ROOT,
    encoding: "utf8",
    shell: false,
    maxBuffer: 20 * 1024 * 1024,
  });

  const combined = [child.stdout ?? "", child.stderr ?? ""].filter(Boolean).join("\n");
  if (combined) process.stdout.write(combined.endsWith("\n") ? combined : `${combined}\n`);

  const parsed = parseCargoTestOutput(combined);
  printSummaryTable(parsed.tests);

  const { summary } = parsed;
  console.log(
    `\nResult: ${summary.result} — ${summary.passed} passed, ${summary.failed} failed, ${summary.ignored} ignored`,
  );

  if (writeReport) {
    const reportPath = path.join(JARVIS_ROOT, "composer-eval-report.json");
    const report = {
      timestamp: new Date().toISOString(),
      passed: summary.passed,
      failed: summary.failed,
      ignored: summary.ignored,
      result: summary.result,
      exitCode: child.status ?? 1,
      tests: parsed.tests,
    };
    fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
    console.log(`\nWrote ${reportPath}`);
  }

  const failed = summary.failed > 0 || summary.result === "FAILED" || (child.status ?? 1) !== 0;
  process.exit(failed ? 1 : 0);
}

runLiveEval();
