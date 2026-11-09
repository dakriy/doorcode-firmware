//! `doorcode-host`: one virtual door on a desktop, controlled through a
//! local HTTP JSON API. See `docs/host.md`.

use std::collections::HashMap;
use std::process::ExitCode;

use host::Options;

const USAGE: &str = "usage: doorcode-host [--provisioning PATH] [--data-dir PATH] \
[--hardware-id ID] [--firmware-version VERSION] [--listen ADDRESS]";

fn main() -> ExitCode {
    let started = options(std::env::args().skip(1)).and_then(|options| host::start(&options));
    let result = started.and_then(|host| {
        eprintln!("control API on http://{}", host.address());
        host.wait()
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("doorcode-host: {error}\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

/// The options the flags give, each defaulted when absent.
fn options(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut flags = flags(args)?;
    let mut flag = |name: &str, default: &str| flags.remove(name).unwrap_or(default.to_owned());
    let options = Options {
        provisioning: flag("--provisioning", "provisioning.json").into(),
        data_dir: flag("--data-dir", ".doorcode").into(),
        firmware_version: flag("--firmware-version", env!("CARGO_PKG_VERSION")),
        listen: flag("--listen", "127.0.0.1:8484"),
        hardware_id: flags.remove("--hardware-id"),
    };
    match flags.into_keys().next() {
        Some(unknown) => Err(format!("unknown flag {unknown}")),
        None => Ok(options),
    }
}

/// The flags given, each with its value.
fn flags(args: impl Iterator<Item = String>) -> Result<HashMap<String, String>, String> {
    let args: Vec<String> = args.collect();
    args.chunks(2)
        .map(|pair| match pair {
            [flag, value] => Ok((flag.clone(), value.clone())),
            _ => Err(format!("{} needs a value", pair[0])),
        })
        .collect()
}
