import Foundation

/// Lightweight HTTP client for management endpoints not covered by the
/// generated OpenAPI client (usage, rate-limits, models).
enum APIClient {
    private static let session: URLSession = {
        let config = URLSessionConfiguration.default
        config.timeoutIntervalForRequest = 5
        return URLSession(configuration: config)
    }()

    private static func get<T: Decodable>(_ path: String) async throws -> T {
        let url = AppEnvironment.shared.baseURL.appendingPathComponent(path)
        let (data, response) = try await session.data(from: url)
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
            throw URLError(.badServerResponse)
        }
        return try JSONDecoder().decode(T.self, from: data)
    }

    // MARK: - Usage

    static func usage() async throws -> UsageSnapshot {
        try await get("v0/management/usage")
    }

    static func usageHistory(
        from: Int64? = nil,
        to: Int64? = nil,
        model: String? = nil
    ) async throws -> UsageHistoryResponse {
        var components = URLComponents(
            url: AppEnvironment.shared.baseURL.appendingPathComponent("v0/management/usage/history"),
            resolvingAgainstBaseURL: false
        )!
        var items: [URLQueryItem] = []
        if let from { items.append(.init(name: "from", value: "\(from)")) }
        if let to { items.append(.init(name: "to", value: "\(to)")) }
        if let model { items.append(.init(name: "model", value: model)) }
        if !items.isEmpty { components.queryItems = items }

        let (data, response) = try await session.data(from: components.url!)
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
            throw URLError(.badServerResponse)
        }
        return try JSONDecoder().decode(UsageHistoryResponse.self, from: data)
    }

    // MARK: - Rate Limits

    static func rateLimits() async throws -> RateLimitsResponse {
        try await get("v0/management/ratelimits")
    }

    // MARK: - Models

    static func models() async throws -> ModelsListResponse {
        try await get("v1/models")
    }
}

// MARK: - Response Types

struct UsageSnapshot: Codable, Sendable {
    let total_requests: UInt64
    let success_requests: UInt64
    let failure_requests: UInt64
    let input_tokens: UInt64
    let output_tokens: UInt64
    let models: [String: ModelStats]
}

struct ModelStats: Codable, Sendable {
    let requests: UInt64
    let success: UInt64
    let failure: UInt64
    let input_tokens: UInt64
    let output_tokens: UInt64
}

struct UsageHistoryResponse: Codable, Sendable {
    let from: Int64
    let to: Int64
    let bucket_seconds: Int64
    let buckets: [UsageBucket]
}

struct UsageBucket: Codable, Sendable {
    let period_start: Int64
    let model: String
    let request_count: UInt64
    let input_tokens: UInt64
    let output_tokens: UInt64
}

struct RateLimitsResponse: Codable, Sendable {
    let providers: [ProviderRateLimits]
}

struct ProviderRateLimits: Codable, Sendable {
    let id: String
    let display_name: String
    let accounts: [AccountRateLimit]
}

struct AccountRateLimit: Codable, Sendable {
    let account_id: String
    let snapshot: RateLimitSnapshot
}

struct RateLimitSnapshot: Codable, Sendable {
    let headers: [String: String]
    let captured_at: UInt64
}

struct ModelsListResponse: Codable, Sendable {
    let object: String
    let data: [ModelEntry]
}

struct ModelEntry: Codable, Sendable, Identifiable {
    let id: String
    let object: String
    let created: Int
    let owned_by: String
}
