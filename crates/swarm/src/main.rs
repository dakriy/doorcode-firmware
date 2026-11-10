//! `doorcode-swarm`: a load test of a doorcode server with many desktop
//! doors. Run it only against a development server. See `docs/testing.md`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;
use std::time::Duration;

use swarm::report::Summary;
use swarm::Options;

const USAGE: &str = "usage: doorcode-swarm --server URL --token TOKEN --doors N --rate R \
--duration SECONDS [--idle SECONDS] [--traffic-doors N --traffic-rate R] [--open-seconds S] \
[--forced] [--provisioning PATH] [--broker-host HOST] [--storm] [--json PATH] [--keep]";

/// Flags that take no value.
const SWITCHES: [&str; 3] = ["--storm", "--keep", "--forced"];

fn main() -> ExitCode {
    let parsed = parse(std::env::args().skip(1).collect());
    let result = parsed
        .map_err(|error| format!("{error}\n{USAGE}"))
        .and_then(|(options, json)| run(&options, json));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("doorcode-swarm: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: &Options, json: Option<PathBuf>) -> Result<(), String> {
    let summary = swarm::run(options)?;
    print!("{}", summary.table());
    write_json(json, &summary)
}

/// The options, and where to write the summary as JSON.
fn parse(args: Vec<String>) -> Result<(Options, Option<PathBuf>), String> {
    let mut flags = Flags(flags(args)?);
    let options = options(&mut flags)?;
    let json = flags.0.remove("--json").map(PathBuf::from);
    flags.done()?;
    Ok((options, json))
}

fn write_json(path: Option<PathBuf>, summary: &Summary) -> Result<(), String> {
    let Some(path) = path else { return Ok(()) };
    fs::write(&path, summary.json()).map_err(|error| format!("{}: {error}", path.display()))
}

fn options(flags: &mut Flags) -> Result<Options, String> {
    let server = flags.required("--server")?;
    let token = flags.required("--token")?;
    let (doors, rate, duration, idle) = numbers(flags)?;
    let (traffic_doors, traffic_rate, open_seconds) = traffic(flags)?;
    Ok(Options {
        server,
        token,
        doors,
        rate,
        traffic_doors,
        traffic_rate,
        open_for: Duration::from_secs(open_seconds),
        forced: flags.0.remove("--forced").is_some(),
        duration: Duration::from_secs(duration),
        idle: Duration::from_secs(idle),
        provisioning: flags.0.remove("--provisioning").map(PathBuf::from),
        broker_host: flags.0.remove("--broker-host"),
        storm: flags.0.remove("--storm").is_some(),
        keep: flags.0.remove("--keep").is_some(),
    })
}

/// The doors, the rate, the duration, and the idle time.
fn numbers(flags: &mut Flags) -> Result<(usize, u32, u64, u64), String> {
    Ok((
        flags.count("--doors", None)?,
        flags.count("--rate", None)?,
        flags.count("--duration", None)?,
        flags.count("--idle", Some(30))?,
    ))
}

/// The traffic doors, their openings per minute, and how long each opening
/// stays open. The rate is needed only with traffic doors.
fn traffic(flags: &mut Flags) -> Result<(usize, u32, u64), String> {
    let doors = flags.whole("--traffic-doors")?;
    Ok((
        doors,
        flags.count("--traffic-rate", (doors == 0).then_some(1))?,
        flags.count("--open-seconds", Some(3))?,
    ))
}

/// The flags given, each with its value; a switch has an empty one.
fn flags(args: Vec<String>) -> Result<HashMap<String, String>, String> {
    let mut flags = HashMap::new();
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let value = if SWITCHES.contains(&flag.as_str()) {
            Some(String::new())
        } else {
            args.next()
        };
        let value = value.ok_or_else(|| format!("{flag} needs a value"))?;
        flags.insert(flag, value);
    }
    Ok(flags)
}

/// The flags not taken yet.
struct Flags(HashMap<String, String>);

impl Flags {
    fn required(&mut self, name: &str) -> Result<String, String> {
        self.0
            .remove(name)
            .ok_or_else(|| format!("{name} is required"))
    }

    /// A count above zero, or `default` when the flag is absent.
    fn count<T: FromStr + Default + PartialOrd>(
        &mut self,
        name: &str,
        default: Option<T>,
    ) -> Result<T, String> {
        let given = self.0.remove(name).map(|text| text.parse().ok());
        let count = given.unwrap_or(default);
        count
            .filter(|count| *count > T::default())
            .ok_or_else(|| format!("{name} needs a whole number above zero"))
    }

    /// A whole number, or 0 when the flag is absent.
    fn whole(&mut self, name: &str) -> Result<usize, String> {
        let Some(text) = self.0.remove(name) else {
            return Ok(0);
        };
        text.parse()
            .map_err(|_| format!("{name} needs a whole number"))
    }

    /// Fails on a flag nothing took.
    fn done(self) -> Result<(), String> {
        match self.0.into_keys().next() {
            Some(unknown) => Err(format!("unknown flag {unknown}")),
            None => Ok(()),
        }
    }
}
