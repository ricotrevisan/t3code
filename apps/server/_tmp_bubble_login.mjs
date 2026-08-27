import { chromium } from "playwright-core";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const CDP = process.env.CDP_URL || "http://127.0.0.1:9222";
const OUT = process.env.SHOT_DIR || "/tmp/bubble-login-shots";
const ITEM = process.env.OP_ITEM_ID || "x2lloqmz5b53rfuqlbfmtvy7f4";
const VAULT = process.env.OP_VAULT || "Dev";
const START_URL = process.env.BUBBLE_URL || "https://bubble.io/login";

fs.mkdirSync(OUT, { recursive: true });
try {
  fs.chmodSync(OUT, 0o700);
} catch {}

function log(msg) {
  console.log(String(msg).slice(0, 500));
}
function pathOnly(u) {
  try {
    const x = new URL(u);
    return x.origin + x.pathname;
  } catch {
    return "invalid-url";
  }
}
function getOtp() {
  const v = execFileSync("op", ["item", "get", ITEM, "--vault", VAULT, "--otp"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  }).trim();
  if (!v) throw new Error("empty otp");
  return v;
}

function describeInputs(page) {
  return page.evaluate(() => {
    const els = [...document.querySelectorAll("input, button, textarea, [role=button]")];
    return els.slice(0, 40).map((el) => ({
      tag: el.tagName,
      type: el.getAttribute("type"),
      name: el.getAttribute("name"),
      id: el.id || null,
      placeholder: el.getAttribute("placeholder"),
      aria: el.getAttribute("aria-label"),
      text: (el.innerText || "").trim().slice(0, 80),
    }));
  });
}

const user = process.env.BUBBLE_USER || "";
const pass = process.env.BUBBLE_PASS || "";
if (!user || !pass) {
  console.error("missing creds");
  process.exit(2);
}
const browser = await chromium.connectOverCDP(CDP);
const context = browser.contexts()[0];
if (!context) {
  console.error("no browser context on CDP");
  process.exit(3);
}
const page = await context.newPage();
page.setDefaultTimeout(30000);
log("navigating");
await page.goto(START_URL, { waitUntil: "domcontentloaded", timeout: 45000 });
await page.waitForTimeout(2500);
await page.screenshot({ path: path.join(OUT, "01-login.png"), fullPage: true });
log("url_after_nav " + pathOnly(page.url()));
log("inputs " + JSON.stringify(await describeInputs(page)));

const emailSel = [
  "input[type=email]",
  "input[name=email]",
  "input[autocomplete=username]",
  "input[placeholder*=email i]",
  "input[name=username]",
].join(", ");
const passSel = [
  "input[type=password]",
  "input[name=password]",
  "input[autocomplete=current-password]",
].join(", ");

async function fillFirst(selector, value) {
  const loc = page.locator(selector).first();
  await loc.waitFor({ state: "visible", timeout: 15000 });
  await loc.click();
  await loc.fill("");
  await loc.fill(value);
}
const emailVisible = await page
  .locator(emailSel)
  .first()
  .isVisible()
  .catch(() => false);
const passVisible = await page
  .locator(passSel)
  .first()
  .isVisible()
  .catch(() => false);
log("email_visible " + emailVisible + " pass_visible " + passVisible);

if (!emailVisible) {
  const bodyText = await page
    .locator("body")
    .innerText()
    .catch(() => "");
  log("body_snip " + bodyText.replace(/\s+/g, " ").slice(0, 400));
  await page.screenshot({ path: path.join(OUT, "02-result.png"), fullPage: true });
  log("FINAL " + pathOnly(page.url()));
  log("STATUS blocked-no-email-field");
  await page.close();
  await browser.close();
  process.exit(4);
}

await fillFirst(emailSel, user);
if (!passVisible) {
  const cont = page.getByRole("button", { name: /continue|next|log in|sign in/i }).first();
  if (await cont.count()) {
    await cont.click();
    await page.waitForTimeout(1500);
  }
}
await fillFirst(passSel, pass);
const submit = page.getByRole("button", { name: /log in|sign in|continue|submit/i }).first();
if (await submit.count()) {
  await submit.click();
} else {
  await page.locator(passSel).first().press("Enter");
}
log("submitted");
await page.waitForTimeout(3000);
await page.screenshot({ path: path.join(OUT, "01b-after-submit.png"), fullPage: true });
log("url_after_submit " + pathOnly(page.url()));
log("inputs_after_submit " + JSON.stringify(await describeInputs(page)));
const otpSel = [
  "input[autocomplete=one-time-code]",
  "input[name*=otp i]",
  "input[name*=totp i]",
  "input[name*=code i]",
  "input[placeholder*=code i]",
  "input[inputmode=numeric]",
  'input[maxlength="6"]',
  "input[aria-label*=code i]",
].join(", ");

async function looksLikeOtp() {
  const u = page.url().toLowerCase();
  if (
    u.includes("two-factor") ||
    u.includes("2fa") ||
    u.includes("mfa") ||
    u.includes("otp") ||
    u.includes("verify")
  )
    return true;
  const t = (
    await page
      .locator("body")
      .innerText()
      .catch(() => "")
  ).toLowerCase();
  if (
    t.includes("two-factor") ||
    t.includes("authenticator") ||
    t.includes("verification code") ||
    t.includes("one-time") ||
    t.includes("6-digit") ||
    t.includes("enter the code")
  )
    return true;
  return await page
    .locator(otpSel)
    .first()
    .isVisible()
    .catch(() => false);
}

async function looksLikeCaptcha() {
  const t = (
    await page
      .locator("body")
      .innerText()
      .catch(() => "")
  ).toLowerCase();
  if (t.includes("captcha") || t.includes("i am not a robot") || t.includes("unusual traffic"))
    return true;
  return (
    (await page
      .locator("iframe[src*=recaptcha], iframe[src*=hcaptcha], iframe[src*=challenges.cloudflare]")
      .count()) > 0
  );
}
async function looksLoggedIn() {
  const u = page.url();
  const p = new URL(u).pathname;
  if (p === "/login" || p.startsWith("/login/") || p.includes("/forgot") || p.includes("/signup"))
    return false;
  const t = (
    await page
      .locator("body")
      .innerText()
      .catch(() => "")
  ).toLowerCase();
  if (t.includes("create an app") || t.includes("my apps") || t.includes("your apps")) return true;
  if (
    p.includes("/home") ||
    p.includes("/account") ||
    p.includes("/site") ||
    p.startsWith("/editor")
  )
    return true;
  if (u.includes("app.bubble.io")) return true;
  if (p === "/account/welcome" || p.startsWith("/page")) return true;
  return false;
}

let status = "unknown";
let otpTried = false;
for (let i = 0; i < 25; i++) {
  await page.waitForTimeout(1000);
  if (await looksLikeCaptcha()) {
    status = "blocked-captcha";
    break;
  }
  if (!otpTried && (await looksLikeOtp())) {
    log("otp_prompt");
    otpTried = true;
    try {
      const code = getOtp();
      const otpBox = page.locator(otpSel).first();
      if (await otpBox.count()) {
        await otpBox.fill(code);
      } else {
        await page.keyboard.type(code);
      }
      const verify = page
        .getByRole("button", { name: /verify|continue|submit|log in|confirm/i })
        .first();
      if (await verify.count()) await verify.click();
      await page.waitForTimeout(3000);
    } catch (e) {
      log("otp_error");
      status = "blocked-otp-failed";
      break;
    }
  }
  if (await looksLoggedIn()) {
    status = "success";
    break;
  }
  const t = (
    await page
      .locator("body")
      .innerText()
      .catch(() => "")
  ).toLowerCase();
  if (
    t.includes("incorrect") ||
    t.includes("invalid password") ||
    t.includes("wrong password") ||
    t.includes("we do not recognize") ||
    t.includes("we don")
  ) {
    status = "blocked-bad-credentials";
    break;
  }
  if (t.includes("passkey") && t.includes("authenticate")) {
    status = "blocked-passkey";
    break;
  }
  if ((t.includes("text message") || t.includes("sms")) && t.includes("code")) {
    status = "blocked-sms";
    break;
  }
}
await page.screenshot({ path: path.join(OUT, "02-result.png"), fullPage: true });
if (status === "unknown") status = (await looksLoggedIn()) ? "success" : "blocked-unknown";
log("FINAL " + pathOnly(page.url()));
log("STATUS " + status);
log("title " + (await page.title().catch(() => "")));
await page.close();
await browser.close();
process.exit(status === "success" ? 0 : 5);
