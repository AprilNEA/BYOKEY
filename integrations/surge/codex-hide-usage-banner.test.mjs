import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const script = readFileSync(
  new URL("./codex-hide-usage-banner.js", import.meta.url),
  "utf8",
);
const module = readFileSync(
  new URL("./codex-hide-usage-banner.sgmodule", import.meta.url),
  "utf8",
);
const pattern = new RegExp(module.match(/\bpattern=([^,\s]+)/)[1]);
const url = "https://chatgpt.com/backend-api/wham/usage";
const quota = {
  rate_limit: {
    allowed: false,
    limit_reached: true,
    primary_window: { used_percent: 100, reset_at: 1900000123 },
    secondary_window: { used_percent: 37, reset_at: 1900080456 },
  },
  credits: { has_credits: false, unlimited: false, balance: "0" },
  spend_control: { limit: 42, used: 17 },
  future_field: { preserve: [null, false, "unchanged"] },
};
const body = JSON.stringify({
  ...quota,
  rate_limit_upsell: {
    banner_type: "workspace_member_usage_limit_reached",
    message: "Contact your admin to increase your usage limit",
  },
});

function rewrite(request = {}, response = {}) {
  let result;
  runInNewContext(script, {
    $request: {
      url,
      method: "GET",
      headers: { "User-Agent": "CodexBrowser Mozilla/5.0" },
      ...request,
    },
    $response: { status: 200, body, ...response },
    console: { log() {} },
    $done(value) {
      result = value;
    },
  });
  // Surge receives a plain object, not the VM's realm-specific prototype.
  return JSON.parse(JSON.stringify(result));
}

for (const request of [
  { url, method: "GET", headers: { "User-Agent": "CodexBrowser Mozilla/5.0" } },
  {
    url: `${url}/subscriptions?source=desktop`,
    method: "POST",
    headers: { "user-agent": "Codex Desktop/1.0" },
  },
]) {
  test(`${request.method} ${request.url} removes only the target upsell`, () => {
    assert.equal(pattern.test(request.url), true);
    const result = rewrite(request);
    assert.deepEqual(
      { ...result, body: JSON.parse(result.body) },
      { body: quota },
    );
  });
}

for (const otherUrl of [
  "https://chatgpt.com/backend-api/codex/responses",
  "https://chatgpt.com.example.org/backend-api/wham/usage",
  `${url}/other`,
]) {
  test(`does not intercept ${otherUrl}`, () => {
    assert.equal(pattern.test(otherUrl), false);
    assert.deepEqual(rewrite({ url: otherUrl }), {});
  });
}

for (const [name, request, response] of [
  ["ordinary browser", { headers: { "User-Agent": "Mozilla/5.0" } }, {}],
  ["CLI client", { headers: { "User-Agent": "codex_cli_rs/1.0" } }, {}],
  ["missing client identity", { headers: {} }, {}],
  ["POST on usage", { method: "POST" }, {}],
  ["GET on subscriptions", { url: `${url}/subscriptions` }, {}],
  ["quota rejection", {}, { status: 429 }],
  ["authentication failure", {}, { status: 401 }],
  ["non-JSON response", {}, { body: "<html>upstream failure</html>" }],
  ["missing banner", {}, { body: JSON.stringify(quota) }],
  [
    "different banner",
    {},
    { body: '{"rate_limit_upsell":{"banner_type":"other_banner"}}' },
  ],
]) {
  test(`leaves the response unchanged for ${name}`, () => {
    assert.deepEqual(rewrite(request, response), {});
  });
}
