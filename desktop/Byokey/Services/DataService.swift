import Connect
import Foundation

@Observable
final class DataService {
    // MARK: - Shared State

    private(set) var providers: [Byokey_Management_ProviderStatus] = []
    private(set) var providerAccounts: [Byokey_Management_ProviderAccounts] = []
    private(set) var usage: Byokey_Management_GetUsageResponse?
    private(set) var history: Byokey_Management_GetUsageHistoryResponse?
    private(set) var rateLimits: Byokey_Management_GetRateLimitsResponse?
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

    private var mgmt: Byokey_Management_ManagementServiceClient {
        let client = ProtocolClient(
            httpClient: URLSessionHTTPClient(),
            config: ProtocolClientConfig(
                host: AppEnvironment.shared.baseURL.absoluteString,
                networkProtocol: .connect,
                codec: JSONCodec()
            )
        )
        return Byokey_Management_ManagementServiceClient(client: client)
    }

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
        let resp = await mgmt.listAccounts(request: .init())
        if let msg = resp.message {
            providerAccounts = msg.providers
        }
    }

    // MARK: - Mutations

    func activateAccount(provider: String, accountId: String) async throws {
        var req = Byokey_Management_ActivateAccountRequest()
        req.provider = provider
        req.accountID = accountId
        let resp = await mgmt.activateAccount(request: req)
        if let error = resp.error {
            throw error
        }
        await reloadAccounts()
    }

    func removeAccount(provider: String, accountId: String) async throws {
        var req = Byokey_Management_RemoveAccountRequest()
        req.provider = provider
        req.accountID = accountId
        let resp = await mgmt.removeAccount(request: req)
        if let error = resp.error {
            throw error
        }
        await reloadAccounts()
    }

    // MARK: - Private

    private func fetchAll() async {
        isLoading = true
        defer { isLoading = false }

        if let msg = (await mgmt.getStatus(request: .init())).message {
            providers = msg.providers
        } else {
            providers = []
        }

        if let msg = (await mgmt.listAccounts(request: .init())).message {
            providerAccounts = msg.providers
        }

        if let msg = (await mgmt.getUsage(request: .init())).message {
            usage = msg
        } else {
            usage = nil
        }

        if let msg = (await mgmt.getRateLimits(request: .init())).message {
            rateLimits = msg
        } else {
            rateLimits = nil
        }

        // /v1/models is still REST — fetch via plain HTTP.
        await fetchModels()

        let now = Int64(Date().timeIntervalSince1970)
        var histReq = Byokey_Management_GetUsageHistoryRequest()
        histReq.from = now - 86400
        histReq.to = now
        if let msg = (await mgmt.getUsageHistory(request: histReq)).message {
            history = msg
        } else {
            history = nil
        }
    }

    private func fetchModels() async {
        let url = AppEnvironment.shared.baseURL.appendingPathComponent("v1/models")
        do {
            let (data, _) = try await URLSession.shared.data(from: url)
            let resp = try JSONDecoder().decode(ModelsResponse.self, from: data)
            models = resp.data
        } catch {
            models = []
        }
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
