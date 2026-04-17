/// Type aliases mapping generated ConnectRPC proto types to short names.

// MARK: - Status service
typealias UsageSnapshot = Byokey_Status_GetUsageResponse
typealias UsageHistoryResponse = Byokey_Status_GetUsageHistoryResponse
typealias UsageBucket = Byokey_Status_UsageBucket
typealias ProtoModelStats = Byokey_Status_ModelStats
typealias RateLimitsResponse = Byokey_Status_GetRateLimitsResponse
typealias ProviderRateLimits = Byokey_Status_ProviderRateLimits
typealias AccountRateLimit = Byokey_Status_AccountRateLimit
typealias ProtoRateLimitSnapshot = Byokey_Status_RateLimitSnapshot
typealias AccountUsageRow = Byokey_Status_AccountUsageRow

// MARK: - Amp service
typealias AmpThreadSummary = Byokey_Amp_ThreadSummary
typealias AmpThreadDetail = Byokey_Amp_ThreadDetail
typealias AmpMessage = Byokey_Amp_Message
typealias AmpContentBlock = Byokey_Amp_ContentBlock
typealias AmpToolUse = Byokey_Amp_ToolUse
typealias AmpToolResult = Byokey_Amp_ToolResult
typealias AmpToolRun = Byokey_Amp_ToolRun
typealias AmpUsage = Byokey_Amp_Usage
typealias AmpMessageState = Byokey_Amp_MessageState

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
