import { expect, test } from "@playwright/test";
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const repo = fileURLToPath(new URL("../../", import.meta.url));
const approvalPreview = ["python3", "-c", "from http.server import HTTPServer,SimpleHTTPRequestHandler;import os;HTTPServer(('127.0.0.1',int(os.environ['SWARM_PORT'])),SimpleHTTPRequestHandler).serve_forever()"];
let dir, service, meta, openEnv;

test.beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "swarm-browser-"));
  const path = join(dir, "profiles.json");
  const fixture = { kind: "custom", executable: join(repo, "tests/fixture-agent.py") };
  const programs = join(dir, "programs");
  mkdirSync(programs);
  const opened = join(dir, "opened");
  for (const name of ["open", "xdg-open"]) {
    writeFileSync(join(programs, name), '#!/bin/sh\nprintf "%s\\n" "$1" >> "$SWARM_TEST_OPENED"\n', { mode: 0o700 });
  }
  const native = join(programs, "fake-codex");
  writeFileSync(native, '#!/bin/sh\nif [ -f "$SWARM_TEST_SIGNED_IN" ]; then echo "Logged in using ChatGPT"; else echo "Not logged in"; exit 1; fi\n', { mode: 0o700 });
  writeFileSync(path, JSON.stringify({ profiles: {
    first: fixture,
    second: { ...fixture, benchmark_model: "Efficient model", benchmark_agent: "Fixture Agent" },
    writer: { ...fixture, benchmark_model: "Frontier model", benchmark_agent: "Fixture Agent" },
    codex: { kind: "codex", executable: native },
    claude: { kind: "claude", executable: join(programs, "missing-claude") },
  } }));
  const now = new Date().toISOString();
  writeFileSync(join(dir, "benchmark-cache.json"), JSON.stringify({ fetched: now, rows: [
    { model: "Frontier model", agent: "Fixture Agent", class: "terminal", suite: "Fixture Bench", date: now.slice(0, 10), score: 91, cost_per_trial: 1, source: "https://example.invalid/benchmark" },
    { model: "Efficient model", agent: "Fixture Agent", effort: "high", class: "terminal", suite: "Fixture Bench", date: now.slice(0, 10), score: 88, cost_per_trial: .1, source: "https://example.invalid/benchmark" },
  ] }));
  openEnv = { ...process.env, SWARM_STATE_DIR: dir, SWARM_TEST_OPENED: opened, SWARM_TEST_SIGNED_IN: join(dir, "signed-in"), SWARM_TEST_LIVE: join(dir, "live.json"), PATH: programs + ":" + process.env.PATH };
  service = spawn(process.env.SWARM_BIN || join(repo, "target/debug/swarm"), ["open"], { env: openEnv, stdio: "ignore" });
  await expect.poll(() => {
    try {
      meta = JSON.parse(readFileSync(join(dir, "service.json"), "utf8"));
      return meta.url;
    } catch {
      return "";
    }
  }).toContain("127.0.0.1");
  await expect.poll(() => {
    try { return readFileSync(opened, "utf8").trim(); } catch { return ""; }
  }).toBe(meta.url + "/#" + meta.token);
  execFileSync(process.env.SWARM_BIN || join(repo, "target/debug/swarm"), ["open"], { env: openEnv, timeout: 5000 });
  expect(readFileSync(opened, "utf8").trim().split("\n")).toEqual(Array(2).fill(meta.url + "/#" + meta.token));
});

test.afterAll(async () => {
  if (service && service.exitCode === null && service.signalCode === null) {
    const exited = new Promise((resolve) => service.once("exit", resolve));
    service.kill();
    await exited;
  }
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test("keeps agent setup readable in both color schemes", async ({ page }) => {
  await page.goto(meta.url + "/#" + meta.token);
  await expect(page.getByRole("heading", { name: "Connect an agent" })).toBeVisible();
  const setup = page.getByRole("button", { name: "Agent setup" });
  await expect(setup).toHaveAttribute("aria-current", "page");
  const html = page.locator("html");
  for (const colorScheme of ["light", "dark"]) {
    await page.emulateMedia({ colorScheme, reducedMotion: "reduce" });
    await expect(html).toHaveCSS("color-scheme", colorScheme);
    expect(await html.evaluate((element) => element.scrollWidth <= innerWidth)).toBe(true);
  }
  await page.keyboard.press("Tab");
  await expect(page.getByRole("link", { name: "Skip to content" })).toBeFocused();
});

test("select roles, review the brief, answer, preview, and land", async ({ page, context }) => {
  const project = join(dir, "quotes");
  mkdirSync(project);
  for (const args of [
    ["init", "-b", "main"],
    ["config", "user.name", "Swarm Browser"],
    ["config", "user.email", "swarm@example.invalid"],
  ]) execFileSync("git", ["-C", project, ...args]);

  await page.goto(meta.url + "/#" + meta.token);
  await expect(page.getByRole("heading", { name: "Connect an agent" })).toBeVisible();
  await expect(page.getByText("Sign in required")).toBeVisible();
  await expect(page.getByRole("link", { name: "Install Claude Code" })).toBeVisible();
  writeFileSync(openEnv.SWARM_TEST_SIGNED_IN, "");
  await page.getByRole("button", { name: "Check again" }).click();
  await expect(page.getByText("Signed in", { exact: true })).toBeVisible();
  const efficient = page.getByRole("listitem").filter({ has: page.getByText("second", { exact: true }) });
  await efficient.getByText("Model and benchmark", { exact: true }).click();
  await efficient.getByLabel("Benchmark effort").fill("high");
  await efficient.getByRole("button", { name: "Save model settings" }).click();
  await expect(efficient.getByText("Saved for new runs.")).toBeVisible();
  await page.getByRole("button", { name: "Use first", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Start a run" })).toBeVisible();
  await expect(page.getByLabel("Coordinator")).toHaveValue("first");
  await expect(page.getByRole("checkbox", { name: "first" })).toBeChecked();
  await expect(page.getByRole("heading", { name: "Start a run" })).toBeFocused();
  await page.getByRole("button", { name: "Needs you" }).focus();
  await page.keyboard.press("Tab");
  await expect(page.getByLabel("Project folder")).toBeFocused();
  await page.getByLabel("Project folder").fill(project);
  await page.getByLabel("What do you want to build?").fill("Add quote export");
  await page.getByRole("button", { name: "Agent setup", exact: true }).first().click();
  await page.getByRole("button", { name: "New run" }).click();
  await expect(page.getByLabel("Project folder")).toHaveValue(project);
  await expect(page.getByLabel("What do you want to build?")).toHaveValue("Add quote export");
  await page.getByLabel("Coordinator").selectOption("first");
  await page.getByRole("checkbox", { name: "first" }).uncheck();
  await page.getByRole("checkbox", { name: "writer" }).check();
  await page.getByRole("checkbox", { name: "second" }).check();
  const routing = page.getByRole("combobox", { name: /Worker routing/ });
  await expect(routing).toHaveValue("manual");
  await routing.selectOption("balanced");
  await page.getByRole("button", { name: "Improve brief" }).click();
  await expect(page.getByLabel("Suggested brief")).toHaveValue(/Keep quotes private/);
  await page.getByRole("button", { name: "Use suggestion" }).click();
  await page.getByText("Original brief", { exact: true }).click();
  await page.getByRole("button", { name: "Restore original" }).click();
  await page.getByRole("button", { name: "Start run", exact: true }).click();
  await page.getByRole("button", { name: "Conversation", exact: true }).click();
  await page.getByLabel("Who can view quotes?").fill("private");
  await page.getByRole("button", { name: "Send answer" }).click();
  const recipe = {
    checks: [["python3", "export.py"]],
    preview: approvalPreview,
  };
  await expect(page.getByRole("heading", { name: "Review acceptance checks" })).toBeVisible();
  await expect(page.getByText('["python3","export.py"]', { exact: true })).toBeVisible();
  await page.getByText("View or edit JSON", { exact: true }).click();
  await page.getByLabel("Acceptance recipe").fill("");
  await page.getByRole("button", { name: "Approve checks" }).click();
  await expect(page.getByRole("alert")).toContainText("Enter a valid JSON object");
  await expect(page.getByLabel("Acceptance recipe")).toHaveValue("");
  await page.getByLabel("Acceptance recipe").fill(JSON.stringify(recipe));
  await page.getByRole("button", { name: "Approve checks" }).click();
  await page.getByRole("button", { name: "Work", exact: true }).click();
  await expect(page.getByRole("button", { name: "Land on main" })).toBeVisible({ timeout: 15_000 });
  await page.getByText("Why this model?", { exact: true }).click();
  await expect(page.getByText(/Efficient model.*88.0%/)).toBeVisible();
  await page.getByRole("button", { name: "Conversation" }).click();
  await page.getByLabel("Message", { exact: true }).fill("Why did the coordinator split the work?");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.getByText("The export milestone owns the generated CSV.")).toBeVisible();
  await page.getByLabel("Message", { exact: true }).fill("Add a timing milestone");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Proposed milestones" })).toBeVisible();
  await page.getByRole("button", { name: "Add milestones" }).click();
  await expect(page.getByText("Milestones added")).toBeVisible();
  await page.getByRole("button", { name: "Work", exact: true }).click();
  await expect(page.getByRole("button", { name: "Land on main" })).toBeVisible({ timeout: 15_000 });
  // Add an event after the server takes the snapshot but before the page sees it.
  let cursorAtReload;
  await page.route("**/api/snapshot", async (route) => {
    const response = await route.fetch();
    const snapshot = await response.json();
    cursorAtReload = snapshot.cursor;
    const changed = await page.request.post(meta.url + "/api/runs/" + snapshot.runs[0].id, {
      headers: { Authorization: "Bearer " + meta.token },
      data: { kind: "chat", text: "Explain the result after reopening." },
    });
    expect(changed.ok()).toBeTruthy();
    await route.fulfill({ response });
  }, { times: 1 });
  const poll = page.waitForRequest((request) => new URL(request.url()).pathname === "/api/events");
  await page.reload();
  const firstPoll = await poll;
  expect(cursorAtReload).toBeGreaterThan(0);
  expect(new URL(firstPoll.url()).searchParams.get("after")).toBe(String(cursorAtReload));
  await page.getByRole("navigation", { name: "Runs" }).getByRole("button", { name: /Add quote export/ }).click();
  await page.getByRole("button", { name: "Conversation" }).click();
  await expect(page.getByText("Milestones added")).toBeVisible();
  await expect(page.getByText("Explain the result after reopening.")).toBeVisible();
  await page.getByRole("button", { name: "Work", exact: true }).click();
  const candidate = page.getByRole("article").filter({ has: page.getByRole("button", { name: "Land on main" }) });
  await candidate.getByRole("button", { name: "View diff" }).click();
  await expect(page.getByLabel("Candidate diff")).toContainText("timing.txt");
  await candidate.getByRole("button", { name: "View checks" }).click();
  await expect(page.getByLabel("Acceptance checks")).toContainText("open quotes");
  await candidate.getByRole("button", { name: "Start preview" }).click();
  const popup = context.waitForEvent("page");
  await page.getByRole("link", { name: /Open preview/ }).click();
  const preview = await popup;
  await expect(preview.getByRole("link", { name: "export.py" })).toBeVisible();
  expect(new URL(preview.url()).origin).not.toBe(meta.url);
  await preview.close();
  await page.getByRole("button", { name: "Land on main" }).click();
  await expect(page.locator("main").getByText("Landed", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Landed commit")).toHaveText(execFileSync("git", ["-C", project, "rev-parse", "HEAD"], { encoding: "utf8" }).trim());
  expect(readFileSync(join(project, "export.py"), "utf8")).toContain("open quotes");
});

test("the workbench requires the session link", async ({ page }) => {
  await page.goto(meta.url);
  await expect(page.getByRole("alert")).toContainText("link printed in Terminal");
});

test("setup exposes its next action and recovers from request failures", async ({ page, isMobile }) => {
  await page.route("**/api/snapshot", (route) => route.fulfill({ status: 503, json: { error: "Service temporarily unavailable" } }), { times: 1 });
  await page.goto(meta.url + "/#" + meta.token);
  await expect(page.getByRole("alert")).toContainText("Couldn’t load Pantheon Local");
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(page.getByRole("navigation", { name: "Workspace" })).toBeVisible();
  await page.route("**/api/setup/status", async (route) => {
    const response = await route.fetch();
    const report = await response.json();
    report.git = false;
    report.profiles.codex.state = "sign-in-unknown";
    await route.fulfill({ json: report });
  }, { times: 1 });
  const setup = page.getByRole("navigation", { name: "Workspace" }).getByRole("button", { name: "Agent setup" });
  const refresh = page.getByRole("button", { name: "Check again" });
  if (await setup.getAttribute("aria-current") === "page") await refresh.click();
  else await setup.click();
  await expect(page.getByRole("alert")).toContainText("Git is required");
  await expect(page.getByText("Sign-in unverified")).toBeVisible();
  await expect(page.getByRole("button", { name: "Use first", exact: true })).toBeDisabled();
  if (isMobile) await refresh.tap();
  else {
    await refresh.focus();
    await page.keyboard.press("Enter");
  }
  await expect(page.getByRole("button", { name: "Use first", exact: true })).toBeEnabled();
  if (!isMobile) await expect(refresh).toBeFocused();

  await page.getByRole("button", { name: "Use first", exact: true }).click();
  await page.getByText("Run options", { exact: true }).click();
  await page.route("**/api/destinations", (route) => route.fulfill({ status: 503, json: { error: "Try again shortly" } }), { times: 1 });
  await page.getByRole("combobox", { name: "Delivery" }).selectOption("deploy");
  await expect(page.getByRole("alert")).toContainText("Couldn’t load destinations");
  await page.route("**/api/destinations", (route) => route.fulfill({ json: ["test-staging"] }), { times: 1 });
  await page.getByRole("button", { name: "Reload destinations" }).click();
  await page.getByRole("combobox", { name: "Destination" }).selectOption("test-staging");
  await expect(page.getByText("The tested commit can land and deploy to this destination automatically.")).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(await page.evaluate(() => innerWidth));
});

test("compare, curate evidence and combine a landed winner", async ({ page }) => {
  const project = join(dir, "experiment-project");
  mkdirSync(project);
  for (const args of [
    ["init", "-b", "main"],
    ["config", "user.name", "Swarm Browser"],
    ["config", "user.email", "swarm@example.invalid"],
  ]) execFileSync("git", ["-C", project, ...args]);
  const tokenHeaders = { Authorization: "Bearer " + meta.token };
  const profilesPath = join(dir, "profiles.json");
  const profiles = JSON.parse(readFileSync(profilesPath));
  profiles.profiles["writer-two"] = profiles.profiles.writer;
  writeFileSync(profilesPath, JSON.stringify(profiles));
  const run = await (await page.request.post(meta.url + "/api/runs", {
    headers: tokenHeaders,
    data: { path: project, prompt: "Measure approaches", coordinator: "first", workers: ["writer", "writer-two"],
      compare: true, delivery: "auto",
      recipe: {
        checks: [["true"]],
        cases: [{ id: "search", driver: join(repo, "tests/experiment-grader.py"), interpreter: "python3", required: true, samples: 2 }],
        rule: { metrics: [{ name: "relevance", unit: "hits", direction: "higher" }, { name: "latency", unit: "ms", direction: "lower" }],
          objective: "relevance", limits: { latency: 5 }, minimum: 1 },
      },
    },
  })).json();
  expect(run.id).toBeTruthy();
  await page.goto(meta.url + "/#" + meta.token);
  await page.getByRole("navigation", { name: "Runs" }).getByRole("button", { name: /Measure approaches/ }).click();
  await page.getByRole("button", { name: "Experiment", exact: true }).click();
  await expect(page.locator(".outcome")).toContainText("Landed", { timeout: 20_000 });
  await expect(page.locator(".outcome")).toContainText("balanced");
  await expect(page.getByText("relevance (hits) by latency (ms)")).toBeVisible();
  const landedCommit = page.getByLabel("Landed commit");
  await expect(landedCommit).toHaveText(/^[a-f0-9]{40}$/);
  await page.getByRole("button", { name: "Work", exact: true }).click();
  await expect(landedCommit).toHaveText(/^[a-f0-9]{40}$/);
  await page.getByRole("button", { name: "Experiment", exact: true }).click();
  await page.getByText("Eval suite", { exact: true }).click();
  await expect(page.getByText("latency must be at most 5 ms")).toBeVisible();
  await page.getByText("Eval suite", { exact: true }).click();
  const plotPoint = page.getByRole("button", { name: "balanced, 7 relevance (hits)" }).first();
  await plotPoint.focus();
  await expect(plotPoint).toBeFocused();
  await expect(plotPoint).toHaveAccessibleDescription("3.05 latency (ms)");
  const baselinePoint = page.getByRole("button", { name: "Baseline, 3 relevance (hits)" });
  expect((await plotPoint.boundingBox()).y).toBeLessThan((await baselinePoint.boundingBox()).y);
  await page.route("**/api/runs/*/evidence/*", (route) => route.fulfill({ status: 503, json: { error: "Evidence unavailable" } }), { times: 1 });
  await page.getByRole("button", { name: "Compare case results" }).click();
  await expect(page.getByRole("rowheader", { name: "search", exact: true })).toBeVisible();
  await expect(page.getByText("Couldn’t load case results for 1 implementation.")).toBeVisible();
  await page.getByRole("button", { name: "Reload case results" }).click();
  await expect(page.getByRole("button", { name: "Reload case results" })).toHaveCount(0);
  await page.getByRole("combobox", { name: "Show", exact: true }).selectOption("Regressions");
  await expect(page.getByText("No regressions in the loaded results.")).toBeVisible();
  await page.getByLabel("Feedback for the coordinator").fill("Exact title lookup should survive.");
  await page.getByRole("button", { name: "Send feedback" }).click();
  await page.getByRole("button", { name: /Needs you · 1/ }).click();
  await expect(page.getByRole("heading", { name: "Should this behavior be required?" })).toBeVisible();
  await page.getByRole("button", { name: "Yes", exact: true }).click();
  await page.getByRole("button", { name: "Send answer" }).click();
  await expect(page.getByText("No open questions.")).toBeVisible();
  await page.getByRole("navigation", { name: "Runs" }).getByRole("button", { name: /Measure approaches/ }).click();
  await page.getByText("Project eval cases", { exact: true }).click();
  await expect(page.getByText("Exact title lookup should survive.")).toBeVisible();
  await page.getByRole("button", { name: "Improve this version" }).click();
  await page.getByRole("checkbox", { name: "fast", exact: true }).check();
  await page.getByLabel("What should carry over?").fill("Keep the faster formatter.");
  const commandURL = meta.url + "/api/runs/" + run.id;
  let changedRevision = false;
  await page.route(commandURL, async (route) => {
    if (!changedRevision && route.request().method() === "POST" && route.request().postDataJSON().kind === "combine") {
      changedRevision = true;
      const reply = await page.request.post(commandURL, { headers: tokenHeaders,
        data: { kind: "chat", text: "Summarize the exporter milestone." } });
      expect(reply.ok()).toBeTruthy();
    }
    await route.continue();
  });
  await page.getByRole("button", { name: "Build combination" }).click();
  await expect(page.getByRole("combobox", { name: "Round", exact: true })).toHaveValue("2", { timeout: 20_000 });
  await expect(page.getByText("From balanced + fast")).toBeVisible();
  await expect(page.getByRole("button", { name: "Resume auto-land" })).toBeVisible();
  await page.getByRole("combobox", { name: "Round", exact: true }).selectOption("1");
  await expect(page.locator(".outcome")).toContainText("Landed");
  await expect(page.locator(".outcome").getByRole("heading")).toHaveText("balanced");
  await page.getByRole("combobox", { name: "Round", exact: true }).selectOption("2");
  await page.getByRole("button", { name: "Keep current version" }).click();
  await expect(page.locator(".outcome")).toContainText("Keeping the baseline");
  await expect(page.locator(".outcome").getByRole("heading")).toHaveText("Baseline");
  await expect(page.getByRole("button", { name: "Land selected on main" })).toHaveCount(0);
  const combinationRow = page.getByRole("row").filter({ hasText: "From balanced + fast" });
  await combinationRow.getByRole("button", { name: "Select", exact: true }).click();
  await expect(page.locator(".outcome")).toContainText("Selected");
  await expect(page.getByRole("button", { name: "Land selected on main" })).toBeVisible();
});

test("review blind previews, select a branch update, and verify the release receipt", async ({ page, context }) => {
  const project = join(dir, "review-and-deliver");
  mkdirSync(project);
  for (const args of [["init", "-b", "main"], ["config", "user.name", "Swarm Browser"], ["config", "user.email", "swarm@example.invalid"]]) {
    execFileSync("git", ["-C", project, ...args]);
  }
  writeFileSync(openEnv.SWARM_TEST_LIVE, JSON.stringify({ digest: "sha256:previous", healthy: true }));
  writeFileSync(join(dir, "destinations.json"), JSON.stringify({
    staging: { executable: join(repo, "tests/fixture-release.py"), environment: ["SWARM_TEST_LIVE"] },
  }));
  const headers = { Authorization: "Bearer " + meta.token };
  const profilesPath = join(dir, "profiles.json");
  const profiles = JSON.parse(readFileSync(profilesPath));
  profiles.profiles["writer-two"] = profiles.profiles.writer;
  writeFileSync(profilesPath, JSON.stringify(profiles));
  const created = await page.request.post(meta.url + "/api/runs", { headers, data: {
    path: project, prompt: "Measure approaches with blind review", coordinator: "first", workers: ["writer", "writer-two"],
    compare: true, delivery: "deploy", destination: "staging",
    recipe: { checks: [["true"]], preview: approvalPreview, rubric: "Which preview best serves the intended search?" },
  } });
  expect(created.ok()).toBeTruthy();
  await page.goto(meta.url + "/#" + meta.token);
  await page.getByRole("navigation", { name: "Runs" }).getByRole("button", { name: /Measure approaches with blind review/ }).click();
  await page.getByRole("button", { name: "Experiment", exact: true }).click();
  await page.getByRole("button", { name: "Blind review", exact: true }).click();
  await page.getByRole("button", { name: "Start review" }).click();
  for (const side of ["A", "B"]) {
    await page.getByRole("button", { name: "Load " + side, exact: true }).click();
    const popup = context.waitForEvent("page");
    await page.getByRole("link", { name: "Open " + side, exact: true }).click();
    const preview = await popup;
    await expect(preview.getByRole("link", { name: "result.txt" })).toBeVisible();
    expect(new URL(preview.url()).origin).not.toBe(meta.url);
    await preview.close();
  }
  await page.getByRole("button", { name: "Tie", exact: true }).click();
  await expect(page.getByText("tie", { exact: true })).toBeVisible();
  await page.getByRole("row", { name: /balanced/ }).getByRole("button", { name: "Select", exact: true }).click();
  await expect(page.getByLabel("Landed commit")).toHaveText(/^[a-f0-9]{40}$/);
  await expect(page.getByText("Live and healthy", { exact: true })).toBeVisible({ timeout: 20_000 });
  const live = JSON.parse(readFileSync(openEnv.SWARM_TEST_LIVE));
  expect(live.healthy).toBe(true);
  await expect(page.getByText(live.digest, { exact: true }).first()).toBeVisible();
});
