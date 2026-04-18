import Connect
import Foundation

@Observable
@MainActor
final class DataService {
    // MARK: - Shared State

    private(set) var providers: [Byokey_Status_ProviderStatus] = []
    private(set) var providerAccounts: [Byokey_Accounts_ProviderAccounts] = []
    private(set) var usage: Byokey_Status_GetUsageResponse?
    private(set) var history: Byokey_Status_GetUsageHistoryResponse?
    private(set) var rateLimits: Byokey_Status_GetRateLimitsResponse?
    private(set) var models: [ModelEntry] = []
    private(set) var ampThreads: [AmpThreadSummary] = []
    private(set) var accountUsage: [AccountUsageRow] = []
    private(set) var isLoading = false

    var isServerReachable = false {
        didSet {
            if isServerReachable, !oldValue {
                startPolling()
            } else if !isServerReachable, oldValue {
                stopPolling()
                // Keep last-known data visible during transient disconnects so
                // UI doesn't flicker. Call `clearAll()` explicitly for a hard reset.
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
    @ObservationIgnored
    private var ampClient: Byokey_Amp_AmpServiceClient!

    init() {
        statusClient = Byokey_Status_StatusServiceClient(client: proto)
        accountsClient = Byokey_Accounts_AccountsServiceClient(client: proto)
        ampClient = Byokey_Amp_AmpServiceClient(client: proto)
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

    // MARK: - Amp Threads

    func reloadAmpThreads(limit: UInt32 = 200, hasMessages: Bool = true) async {
        var req = Byokey_Amp_ListThreadsRequest()
        req.limit = limit
        req.hasMessages_p = hasMessages
        if let msg = (await ampClient.listThreads(request: req)).message,
           msg.threads != ampThreads
        {
            ampThreads = msg.threads
        }
    }

    func fetchThread(id: String) async -> AmpThreadDetail? {
        var req = Byokey_Amp_GetThreadRequest()
        req.id = id
        let resp = await ampClient.getThread(request: req)
        guard let msg = resp.message, msg.hasThread else { return nil }
        return msg.thread
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

    @discardableResult
    func addApiKey(
        provider: String,
        apiKey: String,
        accountId: String? = nil,
        label: String? = nil
    ) async throws -> String {
        var req = Byokey_Accounts_AddApiKeyRequest()
        req.provider = provider
        req.apiKey = apiKey
        if let accountId { req.accountID = accountId }
        if let label { req.label = label }
        let resp = await accountsClient.addApiKey(request: req)
        if let error = resp.error { throw error }
        await reloadAccounts()
        return resp.message?.accountID ?? ""
    }

    @discardableResult
    func importClaudeCode(
        accountId: String? = nil,
        label: String? = nil
    ) async throws -> String {
        var req = Byokey_Accounts_ImportClaudeCodeRequest()
        if let accountId { req.accountID = accountId }
        if let label { req.label = label }
        let resp = await accountsClient.importClaudeCode(request: req)
        if let error = resp.error { throw error }
        await reloadAccounts()
        return resp.message?.accountID ?? ""
    }

    /// Run server-streaming OAuth login. Emits progress events as they arrive.
    /// Throws on failure (terminal `.failed` stage or transport error).
    func login(
        provider: String,
        accountId: String? = nil,
        onEvent: @escaping @Sendable (Byokey_Accounts_LoginEvent) -> Void
    ) async throws {
        var req = Byokey_Accounts_LoginRequest()
        req.provider = provider
        if let accountId { req.accountID = accountId }

        let stream = accountsClient.login(headers: [:])
        try stream.send(req)
        var terminalError: String?
        var sawTerminal = false

        streamLoop: for await result in stream.results() {
            switch result {
            case .headers:
                continue
            case .message(let event):
                onEvent(event)
                if event.stage == .done {
                    sawTerminal = true
                    break streamLoop
                } else if event.stage == .failed {
                    sawTerminal = true
                    terminalError = event.error.isEmpty ? "login failed" : event.error
                    break streamLoop
                }
            case .complete(let code, let error, _):
                if code != .ok, terminalError == nil {
                    terminalError = error?.localizedDescription ?? "login transport error (code: \(code))"
                }
            }
        }
        if terminalError == nil && !sawTerminal {
            terminalError = "login stream ended without terminal event"
        }
        if let terminalError {
            throw NSError(
                domain: "Byokey.Login",
                code: -1,
                userInfo: [NSLocalizedDescriptionKey: terminalError]
            )
        }
        await reloadAccounts()
    }

    /// Inject the proxy URL into the local Amp CLI settings file.
    /// Returns the resolved URL.
    @discardableResult
    func injectAmpUrl(url: String? = nil) async throws -> Byokey_Amp_InjectUrlResponse {
        var req = Byokey_Amp_InjectUrlRequest()
        if let url { req.url = url }
        let resp = await ampClient.injectURL(request: req)
        if let error = resp.error { throw error }
        guard let message = resp.message else {
            throw NSError(
                domain: "Byokey.AmpInject",
                code: -1,
                userInfo: [NSLocalizedDescriptionKey: "empty response"]
            )
        }
        return message
    }

    // MARK: - Private

    private func fetchAll() async {
        // Only show loading state on the first load to prevent flickering on every poll tick.
        let isFirstLoad = providers.isEmpty && providerAccounts.isEmpty
        if isFirstLoad { isLoading = true }
        defer { if isFirstLoad { isLoading = false } }

        // On every field: only reassign when the new value differs from the current one,
        // AND never clobber a previously-successful value with nil/[] on a transient failure.
        if let msg = (await statusClient.getStatus(request: .init())).message,
           msg.providers != providers
        {
            providers = msg.providers
        }

        if let msg = (await accountsClient.listAccounts(request: .init())).message,
           msg.providers != providerAccounts
        {
            providerAccounts = msg.providers
        }

        if let msg = (await statusClient.getUsage(request: .init())).message,
           msg != usage
        {
            usage = msg
        }

        if let msg = (await statusClient.getRateLimits(request: .init())).message,
           msg != rateLimits
        {
            rateLimits = msg
        }

        await fetchModels()

        // NOTE: history is intentionally NOT fetched here.
        // It is exclusively managed by loadHistory(from:to:) which is driven
        // by the UsageRange picker in OverviewView.  Polling must not clobber
        // the user-selected range with a hard-coded 24h window.

        if let msg = (await statusClient.getUsageByAccount(request: .init())).message,
           msg.rows != accountUsage
        {
            accountUsage = msg.rows
        }

        // NOTE: ampThreads are intentionally NOT fetched here.
        // Threads are heavyweight; they are loaded on-demand when ThreadsView
        // appears and on a separate 30-second timer managed by that view.
    }

    private func fetchModels() async {
        let url = AppEnvironment.shared.baseURL.appendingPathComponent("v1/models")
        do {
            let (data, _) = try await URLSession.shared.data(from: url)
            let resp = try JSONDecoder().decode(ModelsResponse.self, from: data)
            if resp.data.map(\.id) != models.map(\.id) {
                models = resp.data
            }
        } catch {
            // Preserve last-known models on transient failure.
        }
    }

    private func clearAll() {
        providers = []
        providerAccounts = []
        usage = nil
        history = nil
        rateLimits = nil
        models = []
        ampThreads = []
        accountUsage = []
    }
}
