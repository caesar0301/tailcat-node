//! tailcat-node: a small cross-platform daemon around Tailcat.
//!
//! `tailcat-node` owns node lifecycle, peer management and
//! agent-level semantics. Tailcat owns encrypted connectivity.
//!
//! Design principle: remain extremely thin. Let Tailcat do
//! networking, let the Agent Runtime do agent semantics, and make
//! `tailcat-node` the small bridge between the two.

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match tailcat_node::cli::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", e.inner_message());
            std::process::ExitCode::from(e.exit_code() as u8)
        }
    }
}
