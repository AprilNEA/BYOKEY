import Combine
import Sparkle

/// Thin @Observable wrapper around Sparkle's SPUStandardUpdaterController,
/// bridging KVO properties into the Observation framework.
@Observable
final class UpdaterState {
    private(set) var canCheckForUpdates = false

    var automaticallyChecksForUpdates: Bool {
        get { controller.updater.automaticallyChecksForUpdates }
        set { controller.updater.automaticallyChecksForUpdates = newValue }
    }

    private let controller: SPUStandardUpdaterController
    private var cancellable: AnyCancellable?

    init() {
        controller = SPUStandardUpdaterController(
            startingUpdater: true,
            updaterDelegate: nil,
            userDriverDelegate: nil
        )
        cancellable = controller.updater.publisher(for: \.canCheckForUpdates)
            .sink { [weak self] value in
                self?.canCheckForUpdates = value
            }
    }

    func checkForUpdates() {
        controller.updater.checkForUpdates()
    }
}
