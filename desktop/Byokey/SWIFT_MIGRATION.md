# Swift desktop app: REST → ConnectRPC migration

The byokey proxy server no longer exposes management endpoints as REST
(`/v0/management/*`). Those endpoints now live at
`/byokey.management.ManagementService/{Method}` over **ConnectRPC**.

The existing Swift code in this folder uses `swift-openapi-generator`
against a now-deleted `openapi.json`. It still compiles but runtime calls
to management methods will fail against the new proxy.

## State after this PR

The Rust server is fully migrated. The Swift app:

- **Compiles and launches** — the old `Generated/Client.swift` +
  `Generated/Types.swift` are still in place, so no build errors.
- **Loses management functionality** — list accounts, usage, ratelimits,
  amp threads all return errors at runtime until the steps below run.
- **Core REST routes still work** — `/v1/chat/completions`, `/v1/messages`,
  `/v1/models`, `/openapi.json` are unchanged. The reachability check in
  `Services/ProcessManager.swift` has been redirected from
  `GET /v0/management/status` to `GET /v1/models`, which remains a REST
  endpoint.

## Steps to finish the migration

### 1. Install codegen prerequisites

```sh
brew install bufbuild/buf/buf          # buf
brew install swift-protobuf            # protoc-gen-swift
# protoc-gen-connect-swift — follow
#   https://github.com/connectrpc/connect-swift#code-generation
```

### 2. Regenerate Swift clients from protobuf

From `desktop/Byokey/`:

```sh
buf generate
```

This reads `buf.gen.yaml`, pulls the proto files from
`../../crates/proto/proto/`, and writes generated Swift into
`Generated/Connect/`.

### 3. Add the Connect Swift Package

In Xcode, add:

- **Package**: `https://github.com/connectrpc/connect-swift`
- **Version**: `1.2.1` or later
- **Products**: `Connect`, `ConnectMocks` (tests only)

### 4. Remove legacy OpenAPI generated code

Once the new Connect files compile:

```sh
rm Generated/Client.swift
rm Generated/Types.swift
```

Remove these Swift package dependencies from the Xcode project (they're
only used by the legacy generated code):
- `swift-openapi-runtime`
- `swift-openapi-urlsession`

### 5. Update call sites

The view/service layer uses `Client`, `Components.Schemas.*`, and
`Operations.*` types from the legacy generated client. Replace them with
the new generated `ManagementServiceClient`.

Files to update:

- `Services/DataService.swift` — construct a `ManagementServiceClient`
  with a connect-swift transport instead of the OpenAPI `Client`.
- `Services/TypeAliases.swift` — replace OpenAPI type aliases with
  ones pointing at the generated `Byokey_Management_*` types.
- `Views/AccountsView.swift` — use `ListAccountsRequest` → `listAccounts()`.
- `Views/UsageView.swift` — use `GetUsageRequest` → `getUsage()`.
- `Services/ProcessManager.swift` — already updated to probe
  `GET /v1/models` (a REST endpoint) for reachability.

Example wiring (pseudo-code):

```swift
import Connect

let protocolClient = ProtocolClient(
    httpClient: URLSessionHTTPClient(),
    config: ProtocolClientConfig(
        host: "http://127.0.0.1:8018",
        networkProtocol: .connect,
        codec: JSONCodec()
    )
)
let mgmt = Byokey_Management_ManagementServiceClient(client: protocolClient)
let response = try await mgmt.getStatus(request: .init())
```

### 6. Verify

```sh
xcodebuild -project desktop/Byokey.xcodeproj -scheme Byokey build
```

Launch the app and confirm the accounts / usage / ratelimits views
populate correctly.
