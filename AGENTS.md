# Cranpose Build

- Implementation, tests and release tools are Rust. UI belongs in Cranpose.
- Use RustRover code intelligence with projectPath; use the IDE-backed helper when direct tools are unavailable.
- Keep platform support explicit. A successful compilation is not a successful launch or native verification.
- Never silently move a build to another machine or accept SDK licenses for the user.
- Preserve application source and release configuration. Build settings belong to this tool's command invocation.
- Avoid unsafe code and unwrap. Test process failures, cancellation, paths with spaces and generated packages.
- Keep tests under tests/. Run cargo fmt, cargo clippy --all-targets -- -D warnings, and cargo test.
- Do not use subagents unless requested.
