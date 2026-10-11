# Codex Desktop usage banner (Surge for Mac)

This optional module hides the workspace-member usage-limit banner in Codex Desktop, including the Codex interface in ChatGPT.app. It is useful when BYOKEY routes inference to another provider but Desktop still displays the official account's quota warning. BYOKEY does not install or enable this module.

The script removes `rate_limit_upsell` only when its `banner_type` is `workspace_member_usage_limit_reached`. It processes HTTP 200 JSON responses for these endpoints:

- `GET https://chatgpt.com/backend-api/wham/usage`
- `POST https://chatgpt.com/backend-api/wham/usage/subscriptions`

Query strings are allowed. Requests must identify as `CodexBrowser …` or `Codex Desktop/…` in `User-Agent`. Other clients, other banners, non-JSON responses, and error responses remain unchanged. Real quotas, reset times, and other response fields remain unchanged. The module does not modify `app.asar`, inference requests, or approval requests.

**This is not a quota bypass.** Usage data is account-wide and does not identify the selected model or provider. The banner is hidden for every model, including official ChatGPT models. Actual quota errors still apply. These are private client endpoints; client updates can stop the module from working.

## Security and scope

- **MITM decrypts HTTPS traffic for the whole `chatgpt.com` hostname**, not just the two script endpoints. The script's response-matching conditions do not restrict the TLS decryption scope.
- **Surge Logbook can retain authentication headers.** During local validation, script input records contained authentication headers even with `debug` disabled. The script's own messages do not print credentials or response bodies, but disabling script debugging does not prevent Surge's input recording. Do not export or share those records. Do not enable this module if that retention is unacceptable.
- The module uses `hostname = %APPEND% chatgpt.com` to preserve existing MITM hostnames. Before enabling MITM, review the complete effective hostname list, including other modules. Enabling the global switch also enables decryption for those existing entries. Do not add a wildcard or disable server certificate verification to make this module work.
- Trust only your own Surge CA. Review both source files before installing. This module uses a local script and does not download script updates automatically.

## Install

These files are independent of BYOKEY release binaries. Both download links below are pinned to the same tested repository revision.

1. Download [codex-hide-usage-banner.sgmodule](https://raw.githubusercontent.com/AprilNEA/BYOKEY/a976730339d8603d8f90d9f96e34a807b66d6add/integrations/surge/codex-hide-usage-banner.sgmodule) and [codex-hide-usage-banner.js](https://raw.githubusercontent.com/AprilNEA/BYOKEY/a976730339d8603d8f90d9f96e34a807b66d6add/integrations/surge/codex-hide-usage-banner.js). Save the raw contents with these exact filenames, not as HTML pages or `.txt` files.
2. Open Surge's profile directory. Place both files beside the active profile, without an extra subdirectory. The module's relative `script-path` expects the JavaScript file there. Back up any existing files with these names before replacing them.
3. Enable **Codex Desktop Usage Banner** in Surge's module list. Enable the **Scripting** switch.
4. Configure and trust your own Surge MITM CA if you have not already done so. Reuse an existing trusted CA. Keep server certificate verification enabled.
5. Review the effective MITM hostname list, then enable the **MITM** switch. Confirm `chatgpt.com` is included and is not excluded by another hostname rule. Do not add `debug=true`.
6. After active tasks finish, fully quit Codex Desktop or ChatGPT.app with `⌘Q`, then reopen it to refresh usage data and HTTPS connections.

The target banner should disappear while the composer and model, effort, and approval controls remain available. An absent banner does not prove that inference works or that the account has quota. Check real provider errors separately. If TLS errors appear, disable the module; do not turn off server certificate verification.

To update, review the source changes and replace both files from the same repository revision. Installing a new BYOKEY binary does not update this module.

## Remove

Disable **Codex Desktop Usage Banner**, then fully quit and reopen the client. Delete the two installed files if no longer needed. Disabling the module does not delete Surge's existing Logbook records or remove its trusted CA.

If MITM was enabled only for this module, turn off the global MITM switch. Otherwise, review the effective hostnames: another profile entry or module may still include `chatgpt.com`.

## Development checks

From the repository root, use the tools provided by `devenv shell`:

```sh
devenv shell biome check integrations/surge
devenv shell node --test integrations/surge/codex-hide-usage-banner.test.mjs
```

The tests execute the script with synthetic Surge request and response objects through its `$done` interface. They check both endpoint forms, quota preservation, and unchanged responses outside the intended scope. They require no credentials or network access. They do not validate TLS interception or the current Desktop UI; check those separately on a Mac before changing client-matching behavior.
