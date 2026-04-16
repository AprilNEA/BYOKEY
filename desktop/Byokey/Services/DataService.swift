import Connect
import Foundation

@Observable
final class DataService {
    // MARK: - Shared State

    private(set) var providers: [Byokey_Status_ProviderStatus] = []
    private(set) var providerAccounts: [Byokey_Accounts_ProviderAccounts] = []
    private(set) var usage: Byokey_Status_GetUsageResponse?
    private(set) var history: Byokey_Status_GetUsageHistoryResponse?
    private(set) var rateLimits: Byokey_Status_GetRateLimitsResponse?
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

    // Stored once — @Observable forbids `lazy var` (the macro turns them
    // into computed properties).  Marked @ObservationIgnored so the macro
    // leaves them as plain stored properties.
    @ObservationIgnored
    private var proto: ProtocolClient = {
        ProtocolClient(
            httpClient: URLSessionHTTPClient(),
            config: ProtocolClientConfig(
                host: AppEnvironment.shared.baseURL.absoluteString,
                networkProtocol: .connect,
                codec: JSONCodec()
            )
        )
    }()
    @ObservationIgnored
    private var statusClient: Byokey_Status_StatusServiceClient!
    @ObservationIgnored
    private var accountsClient: Byokey_Accounts_AccountsServiceClient!

    init() {
        statusClient = Byokey_Status_StatusServiceClient(client: proto)
        accountsClient = Byokey_Accounts_AccountsServiceClient(client: proto)
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

    func loadHistory(from: Int64, to: Int64) async {
        var req = Byokey_Status_GetUsageHistoryRequest()
        req.from = from
        req.to = to
        history = (await statusClient.getUsageHistory(request: req)).message
    }

    func reloadAccounts() async {
        if let msg = (await accountsClient.listAccounts(request: .init())).message {
            providerAccounts = msg.providers
        }
    }

    // MARK: - Mutations

    func activateAccount(provider: String, accountId: String) async throws {
        var req = Byokey_Accounts_ActivateAccountRequest()
        req.provider = provider
        req.accountID = accountId
        let resp = await accountsClient.activateAccount(request: req)
        if let error = resp.error { throw error }
        await reloadAccounts()
    }

    func removeAccount(provider: String, accountId: String) async throws {
        var req = Byokey_Accounts_RemoveAccountRequest()
        req.provider = provider
        req.accountID = accountId
        let resp = await accountsClient.removeAccount(request: req)
        if let error = resp.error { throw error }
        await reloadAccounts()
    }

    // MARK: - Private

    private func fetchAll() async {
        isLoading = true
        defer { isLoading = false }

        if let msg = (await statusClient.getStatus(request: .init())).message {
            providers = msg.providers
        } else {
            providers = []
        }

        if let msg = (await accountsClient.listAccounts(request: .init())).message {
            providerAccounts = msg.providers
        }

        if let msg = (await statusClient.getUsage(request: .init())).message {
            usage = msg
        } else {
            usage = nil
        }

        if let msg = (await statusClient.getRateLimits(request: .init())).message {
            rateLimits = msg
        } else {
            rateLimits = nil
        }

        await fetchModels()

        let now = Int64(Date().timeIntervalSince1970)
        var histReq = Byokey_Status_GetUsageHistoryRequest()
        histReq.from = now - 86400
        histReq.to = now
        if let msg = (await statusClient.getUsageHistory(request: histReq)).message {
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
