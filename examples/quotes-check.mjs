import { chromium } from "../web/node_modules/playwright/index.mjs";
import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import net from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const data = await mkdtemp(join(process.env.SWARM_DATA_DIR || tmpdir(), "quotes-"));
const probe = net.createServer();
await new Promise((resolve, reject) => {
  probe.once("error", reject);
  probe.listen(0, "127.0.0.1", resolve);
});
const port = probe.address().port;
await new Promise((resolve) => probe.close(resolve));
const url = "http://127.0.0.1:" + port;
let app;
let browser;
let context;

async function start() {
  app = spawn("python3", ["app.py"], {
    env: { ...process.env, SWARM_PORT: String(port), SWARM_DATA_DIR: data },
    stdio: ["ignore", "inherit", "inherit"],
  });
  for (let elapsed = 0; elapsed < 10_000; elapsed += 200) {
    if (app.exitCode !== null) throw new Error("app.py exited during startup");
    const ready = await fetch(url, { signal: AbortSignal.timeout(300) }).catch(() => null);
    if (ready?.ok) return;
    await delay(200);
  }
  throw new Error("The app did not respond on SWARM_PORT");
}

async function stop() {
  if (!app || app.exitCode !== null || app.signalCode !== null) return;
  const stopped = new Promise((resolve) => app.once("exit", resolve));
  app.kill();
  await stopped;
}

async function outstanding(page, expected) {
  await page.getByText("Outstanding: " + expected, { exact: true }).waitFor();
  console.log("Verified outstanding total:", expected);
}

try {
  await start();
  browser = await chromium.launch(process.argv[2] ? { executablePath: process.argv[2] } : {});
  context = await browser.newContext();
  await context.tracing.start({ screenshots: true, snapshots: true });
  const page = await context.newPage();
  page.setDefaultTimeout(5_000);
  await page.goto(url);
  await outstanding(page, "$0.00");
  for (const [client, amount] of [["North", "125"], ["South", "75"]]) {
    await page.getByLabel("Client", { exact: true }).fill(client);
    await page.getByLabel("Amount", { exact: true }).fill(amount);
    await page.getByRole("button", { name: "Add quote", exact: true }).click();
  }
  await outstanding(page, "$200.00");
  await page.getByRole("row", { name: /North/ }).getByRole("button", { name: "Accept", exact: true }).click();
  await outstanding(page, "$75.00");
  await stop();
  await start();
  await page.reload();
  await page.getByRole("row", { name: /North.*Accepted/ }).waitFor();
  await outstanding(page, "$75.00");
  await page.screenshot({ path: join(process.env.SWARM_DATA_DIR || data, "quotes.png") });
} finally {
  if (context) await context.tracing.stop({ path: join(process.env.SWARM_DATA_DIR || data, "quotes-trace.zip") });
  if (browser) await browser.close();
  await stop();
  await rm(data, { recursive: true, force: true });
}
