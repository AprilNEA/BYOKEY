# AGENTS.md — byokey-tui

## Boundary
`byokey-tui` is an upper-layer management client crate. It must fetch BYOKEY state through the ConnectRPC management API using `byokey-proto`'s `client` feature.

Do not add dependencies on `byokey-daemon`, `byokey-auth`, or `byokey-store` here. The TUI must not read `SQLite` directly or call daemon process/control internals for status; those details belong behind `proxy`'s management API.
