/// Type aliases mapping generated ConnectRPC / proto types to short names
/// used by the view layer. Management types come from `management.pb.swift`;
/// the models endpoint is still REST so its types are hand-defined here.

// MARK: - ConnectRPC proto type aliases

typealias UsageSnapshot = Byokey_Management_GetUsageResponse
typealias UsageHistoryResponse = Byokey_Management_GetUsageHistoryResponse
typealias UsageBucket = Byokey_Management_UsageBucket
typealias ProtoModelStats = Byokey_Management_ModelStats
typealias RateLimitsResponse = Byokey_Management_GetRateLimitsResponse
typealias ProviderRateLimits = Byokey_Management_ProviderRateLimits
typealias AccountRateLimit = Byokey_Management_AccountRateLimit
typealias ProtoRateLimitSnapshot = Byokey_Management_RateLimitSnapshot

// MARK: - REST-only types (v1/models — not in proto)

struct ModelsResponse: Decodable {
    let object: String
    let data: [ModelEntry]
}

struct ModelEntry: Decodable, Identifiable {
    let id: String
    let object: String
    let owned_by: String
}

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
