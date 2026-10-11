// Hide one account-wide presentation field; preserve quotas and upstream errors.
function rewriteUsageBanner() {
  const route =
    /^https:\/\/chatgpt\.com\/backend-api\/wham\/usage(\/subscriptions)?(?:\?.*)?$/.exec(
      $request.url,
    );
  if (
    !route ||
    $request.method !== (route[1] ? "POST" : "GET") ||
    $response.status !== 200
  ) {
    console.log(
      `Codex usage banner: skipped; route=${Boolean(route)}, method=${Boolean(route && $request.method === (route[1] ? "POST" : "GET"))}, status=${$response.status} (${typeof $response.status}).`,
    );
    return {};
  }

  const userAgent = Object.entries($request.headers ?? {}).find(
    ([name]) => name.toLowerCase() === "user-agent",
  )?.[1];
  if (
    !userAgent?.startsWith("Codex Desktop/") &&
    !userAgent?.startsWith("CodexBrowser ")
  ) {
    console.log(
      `Codex usage banner: skipped; Desktop identity not matched, User-Agent present=${userAgent !== undefined}.`,
    );
    return {};
  }

  let usage;
  try {
    usage = JSON.parse($response.body);
  } catch {
    // A changed upstream format must not interrupt quota updates.
    console.log("Codex usage banner: non-JSON response left unchanged.");
    return {};
  }
  if (
    usage?.rate_limit_upsell?.banner_type !==
    "workspace_member_usage_limit_reached"
  ) {
    console.log("Codex usage banner: skipped; target banner absent.");
    return {};
  }

  delete usage.rate_limit_upsell;
  console.log("Codex usage banner: removed target banner; quotas unchanged.");
  return { body: JSON.stringify(usage) };
}

$done(rewriteUsageBanner());
