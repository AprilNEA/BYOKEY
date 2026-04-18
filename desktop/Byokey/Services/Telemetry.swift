import Foundation
import Sentry

/// Sentry integration for the Byokey desktop app.
///
/// The desktop app talks to the local Rust daemon over ConnectRPC on
/// 127.0.0.1 — none of those requests carry user prompts (only management
/// calls: status, accounts, amp). Still, we strip URL queries and request
/// bodies from breadcrumbs defensively in case future code paths change.
enum Telemetry {
    /// Info.plist key whose value is the Sentry DSN (injected at build
    /// time via xcconfig / GitHub Actions secret). Leave empty or remove
    /// to disable error reporting.
    static let dsnInfoPlistKey = "BYOKEY_SENTRY_DSN"

    /// Starts Sentry if a DSN is configured in Info.plist. No-op otherwise.
    static func start() {
        guard
            let dsn = Bundle.main.object(forInfoDictionaryKey: dsnInfoPlistKey) as? String,
            !dsn.isEmpty,
            // Xcode leaves the literal "$(BYOKEY_SENTRY_DSN)" in the plist
            // when the build setting is unset; treat that as disabled.
            !dsn.hasPrefix("$(")
        else { return }

        SentrySDK.start { options in
            options.dsn = dsn
            options.releaseName = Self.releaseName()
            options.environment = Self.environmentName()
            options.sendDefaultPii = false
            // Performance tracing disabled: we don't need spans for a
            // local-only RPC client and want a smaller event volume.
            options.tracesSampleRate = 0.0
            // Automatic breadcrumbs for URL sessions are fine — our requests
            // are all to 127.0.0.1. Scrub defensively below anyway.
            options.beforeBreadcrumb = { crumb in
                scrubBreadcrumb(crumb)
                return crumb
            }
            options.beforeSend = { event in
                scrubEvent(event)
                return event
            }
        }
    }

    private static func releaseName() -> String {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.0.0"
        let build = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "0"
        return "byokey-desktop@\(version)+\(build)"
    }

    private static func environmentName() -> String {
        #if DEBUG
            return "development"
        #else
            return AppEnvironment.isDev ? "development" : "production"
        #endif
    }

    private static func scrubBreadcrumb(_ crumb: Breadcrumb) {
        // Strip query strings from HTTP breadcrumb URLs.
        if var data = crumb.data, let raw = data["url"] as? String {
            if let qIndex = raw.firstIndex(of: "?") {
                data["url"] = String(raw[..<qIndex])
                crumb.data = data
            }
        }
        // Never forward request or response bodies.
        if var data = crumb.data {
            data.removeValue(forKey: "request_body")
            data.removeValue(forKey: "response_body")
            data.removeValue(forKey: "body")
            crumb.data = data
        }
    }

    private static func scrubEvent(_ event: Event) {
        // Clear request body / query string if any integration ever sets it.
        if let request = event.request {
            request.data = nil
            request.queryString = nil
            request.cookies = nil
            // Strip non-standard auth headers not in Sentry's default list.
            if var headers = request.headers {
                let extraSensitive: Set<String> = [
                    "x-goog-api-key",
                    "x-amp-token",
                    "api-key",
                    "anthropic-version",
                    "openai-organization",
                    "openai-project",
                    "x-session-id",
                ]
                headers = headers.filter { !extraSensitive.contains($0.key.lowercased()) }
                request.headers = headers
            }
        }
    }
}
