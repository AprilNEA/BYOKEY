/// Type aliases mapping old hand-written types to the generated OpenAPI types.
/// This lets Views continue using short names (e.g. `UsageSnapshot`)
/// while all data comes from the generated `Components.Schemas` namespace.

typealias UsageSnapshot = Components.Schemas.UsageSnapshot
typealias UsageHistoryResponse = Components.Schemas.UsageHistoryResponse
typealias UsageBucket = Components.Schemas.UsageBucket
typealias ModelStats = Components.Schemas.ModelStats
typealias ModelEntry = Components.Schemas.ModelEntry
typealias ModelsResponse = Components.Schemas.ModelsResponse

extension Components.Schemas.ModelEntry: Identifiable {}
typealias RateLimitsResponse = Components.Schemas.RateLimitsResponse
typealias ProviderRateLimits = Components.Schemas.ProviderRateLimits
typealias AccountRateLimit = Components.Schemas.AccountRateLimit
typealias RateLimitSnapshot = Components.Schemas.RateLimitSnapshot

// MARK: - Provider Icon

/// Maps a provider id string to its asset catalog image name.
/// Falls back to nil when no custom icon is available.
func providerIconName(for id: String) -> String? {
    switch id.lowercased() {
    case "claude":       "provider-claude"
    case "codex":        "provider-codex"
    case "gemini":       "provider-gemini"
    case "copilot":      "provider-copilot"
    case "kiro":         "provider-kiro"
    case "antigravity":  "provider-antigravity"
    case "qwen":         "provider-qwen"
    case "kimi":         "provider-kimi"
    case "iflow":        "provider-iflow"
    case "amp":          "provider-amp"
    default:             nil
    }
}
