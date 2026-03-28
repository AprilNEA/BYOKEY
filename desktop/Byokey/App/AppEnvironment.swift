import Foundation
import SwiftUI

@Observable
final class AppEnvironment {
    static let shared = AppEnvironment()

    static let bundleIdentifier = Bundle.main.bundleIdentifier ?? "io.byokey.desktop"
    static let isDev: Bool = bundleIdentifier.hasSuffix(".dev")
    static let defaultPort: Int = isDev ? 8019 : 8018

    var port: Int = AppEnvironment.defaultPort

    var baseURL: URL { URL(string: "http://127.0.0.1:\(port)")! }
}
