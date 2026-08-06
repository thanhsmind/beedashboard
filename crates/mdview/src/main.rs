//! mdview — multi-project markdown viewer for AI agent workflows.

mod cli;
mod doctor;
mod herdr;
mod mcp;
mod notify;
mod runtime;
mod server;
mod supervisor;
mod terminal_auth;
mod views;
mod watch;
mod watcher;

use clap::Parser;

fn main() {
    // MCP speaks JSON-RPC on stdout; keep tracing on stderr and quiet by default.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mdview=info,warn".into()),
        )
        .init();

    let cli = cli::Cli::parse();
    if let Err(e) = cli::run(cli) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod inert_until_switched_on {
    //! D7 boundary for agent-terminal-17: this cell only wires the
    //! watcher/supervisor/notify modules into the binary — it never starts
    //! them. Nothing may spawn a process, open a socket on a timer, or make
    //! an outbound request as a result of this cell; the switches that start
    //! them arrive in a later cell. Verified by scanning this file's own
    //! source rather than by observing runtime behavior, since "nothing
    //! happens" has no positive event to assert on.
    const SOURCE: &str = include_str!("main.rs");

    /// Only the production code above this very test module — excluding this
    /// guard's own text, which necessarily names the calls it forbids.
    fn production_source() -> &'static str {
        SOURCE
            .split_once("mod inert_until_switched_on")
            .map(|(before, _)| before)
            .expect("this test module must exist verbatim in main.rs")
    }

    #[test]
    fn main_declares_the_new_modules() {
        let src = production_source();
        for m in ["mod notify;", "mod watcher;", "mod supervisor;"] {
            assert!(src.contains(m), "main.rs must declare `{m}`");
        }
    }

    #[test]
    fn main_never_constructs_or_starts_them() {
        let src = production_source();
        for call in [
            "Supervisor::new(",
            "PollWatcher::new(",
            "NotifyService::new(",
            "TelegramNotifier::new(",
            "SpawnHerdr {",
        ] {
            assert!(
                !src.contains(call),
                "main.rs must not start the watcher/supervisor/notifier yet (found `{call}`); \
                 that wiring belongs to a later, opt-in-switched cell (D7)"
            );
        }
    }
}
