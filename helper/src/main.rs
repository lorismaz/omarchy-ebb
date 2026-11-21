//! helio-strap: the Bluetooth helper for the Helio Strap Omarchy plugin.
//!
//!   helio-strap stream [name-filter] [reconnect: 1|0] [confirmed-address]
//!                                                      JSON lines for the panel;
//!                                                      notification requests on stdin
//!   helio-strap setup [name-filter]                     save the strap's key
//!
//! Written from PROTOCOL.md. Set HELIO_DEBUG=1 to log protocol traffic to stderr.

// Style only: nested ifs read better here than let-chains, and chunks_exact
// is as clear as as_chunks.
#![allow(clippy::collapsible_if, clippy::chunks_exact_to_as_chunks)]

mod account;
mod curve;
mod frame;
mod keyring;
mod notify;
mod proto;
mod session;
mod setup;
mod stream;

pub fn debug(msg: &str) {
    if std::env::var_os("HELIO_DEBUG").is_some() {
        eprintln!("{msg}");
    }
}

const USAGE: &str = "usage: helio-strap stream [name-filter] [1|0] [AA:BB:CC:DD:EE:FF]\n       helio-strap setup [name-filter]";

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize, default: &str| args.get(i).cloned().unwrap_or_else(|| default.to_owned());
    let code = match args.first().map(String::as_str) {
        Some("stream") => {
            // An address the user confirmed in the panel; anything else means none.
            let address = args.get(3).and_then(|a| a.parse::<bluer::Address>().ok());
            let run = stream::run(arg(1, "Helio"), arg(2, "1") != "0", address);
            tokio::spawn(notify::serve());
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal handler");
            tokio::select! {
                code = run => code,
                _ = term.recv() => 0,
                _ = tokio::signal::ctrl_c() => 0,
            }
        }
        Some("setup") => setup::run(arg(1, "")).await,
        Some("--version") => {
            println!("helio-strap {}", env!("CARGO_PKG_VERSION"));
            0
        }
        _ => {
            eprintln!("{USAGE}");
            2
        }
    };
    std::process::exit(code);
}
