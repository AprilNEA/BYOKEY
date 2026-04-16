//! Generate `ConnectRPC` service code from `proto/` at build time.
//!
//! Requires `protoc` on `PATH` (or `PROTOC` env var). Output lands in
//! `$OUT_DIR/_connectrpc.rs` and is `include!`d from `src/lib.rs`.

fn main() {
    println!("cargo:rerun-if-changed=proto");

    connectrpc_build::Config::new()
        .files(&[
            "proto/status.proto",
            "proto/accounts.proto",
            "proto/amp.proto",
        ])
        .includes(&["proto"])
        .include_file("_connectrpc.rs")
        .compile()
        .expect("connectrpc codegen failed");
}
