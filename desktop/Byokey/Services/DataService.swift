import Foundation
import OpenAPIURLSession

@Observable
final class DataService {
    // MARK: - Shared State

    private(set) var providers: [Components.Schemas.ProviderStatus] = []
    private(set) var providerAccounts: [Components.Schemas.ProviderAccounts] = []
    private(set) var usage: UsageSnapshot?
    private(set) var history: UsageHistoryResponse?
    private(set) var rateLimits: RateLimitsResponse?
    private(set) var models: [ModelEntry] = []
    private(set) var isLoading = false

    var isServerReachable = false {
        didSet {
            if isServerReachable, !oldValue {
                startPolling()
            } else if !isServerReachable {
                stopPolling()
                clearAll()
            }
        }
    }

    private var pollTask: Task<Void, Never>?

    // MARK: - Polling

    func startPolling() {
        pollTask?.cancel()
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                await self.fetchAll()
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }

    func stopPolling() {
        pollTask?.cancel()
        pollTask = nil
    }

    func reload() async {
        await fetchAll()
    }

    func reloadAccounts() async {
        let client = Client(
            serverURL: AppEnvironment.shared.baseURL,
            transport: URLSessionTransport()
        )
        do {
            let response = try await client.accounts_handler()
            providerAccounts = try response.ok.body.json.providers
        } catch {
            // keep existing data on error
        }
    }

    // MARK: - Mutations

    func activateAccount(provider: String, accountId: String) async throws {
        let client = Client(
            serverURL: AppEnvironment.shared.baseURL,
            transport: URLSessionTransport()
        )
        _ = try await client.activate_account_handler(
            path: .init(provider: provider, account_id: accountId)
        )
        await reloadAccounts()
    }

    func removeAccount(provider: String, accountId: String) async throws {
        let client = Client(
            serverURL: AppEnvironment.shared.baseURL,
            transport: URLSessionTransport()
        )
        _ = try await client.remove_account_handler(
            path: .init(provider: provider, account_id: accountId)
        )
        await reloadAccounts()
    }

    // MARK: - Private

    private func fetchAll() async {
        isLoading = true
        defer { isLoading = false }

        let baseURL = AppEnvironment.shared.baseURL
        let client = Client(serverURL: baseURL, transport: URLSessionTransport())

        do {
            let resp = try await client.status_handler()
            providers = try resp.ok.body.json.providers
        } catch {
            providers = []
        }

        do {
            let resp = try await client.accounts_handler()
            providerAccounts = try resp.ok.body.json.providers
        } catch {
            // keep existing
        }

        usage = try? await APIClient.usage()
        rateLimits = try? await APIClient.rateLimits()
        models = (try? await APIClient.models())?.data ?? []

        let now = Int64(Date().timeIntervalSince1970)
        history = try? await APIClient.usageHistory(from: now - 86400, to: now)
    }

    private func clearAll() {
        providers = []
        providerAccounts = []
        usage = nil
        history = nil
        rateLimits = nil
        models = []
    }
}
