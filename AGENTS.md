# Cranpose Build

- Implementation, tests and release tools are Rust. UI belongs in Cranpose.
- Use RustRover code intelligence with projectPath; use the IDE-backed helper when direct tools are unavailable.
- Keep platform support explicit. A successful compilation is not a successful launch or native verification.
- Never silently move a build to another machine or accept SDK licenses for the user.
- Preserve application source and release configuration. Build settings belong to this tool's command invocation.
- Avoid unsafe code and unwrap. Test process failures, cancellation, paths with spaces and generated packages.
- Keep tests under tests/. Run cargo fmt, cargo clippy --all-targets -- -D warnings, and cargo test.
- Do not use subagents unless requested.

## Simple speech

Never leave prepositions trailing at the ends of clauses (e.g., use "the version by which..." instead of "the version... by"), and keep modifiers close to the words they modify. Use simple words in their original meaning. no poetic, no jargon. No contrastive sentences. No special symbols. No Trailing Participial Phrases. NO for any of these: Overused Buzzwords, Empty Transition Openers, unnecessary adjectives that try to sell an ordinary fact, list things in triples (e.g., "fast, efficient, and reliable" or "streamline, optimize, and scale")., Not only... but also..., wrap-up summaries that don't add actual data. | **No sales pitch or hype:** Adopt a neutral, engineering-first tone. Never use marketing fluff, exclamation points for enthusiasm, or words designed to "sell" a feature (e.g., *effortless, supercharge, magical, lightning-fast*). State facts and mechanics directly.
