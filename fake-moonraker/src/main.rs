use fake_moonraker::access::AccessConfig;
use fake_moonraker::machine::MachineProfile;
use fake_moonraker::server::{serve, ServerConfig};
use std::process::ExitCode;
use std::time::Duration;
use tokio::net::TcpListener;

const DEFAULT_PORT: u16 = 7125;
const DEFAULT_TIME_SCALE: f64 = 10.0;
const DEFAULT_MMU_GATES: usize = 4;
const USAGE: &str = "Usage: fake-moonraker [--port <port>] [--time-scale <factor>] \
    [--profile basic|voron|happy-hare|ercf|toolchanger|toolchanger-required|happy-hare-units|toolchanger-mmu|afc|afc-toolchanger] \
    [--gates <1-16>, only for happy-hare and ercf] [--auth <user>:<password>]";

enum Profile {
    Basic,
    Voron,
    HappyHare,
    Ercf,
    Toolchanger,
    ToolchangerRequired,
    HappyHareUnits,
    ToolchangerMmu,
    Afc,
    AfcToolchanger,
}

struct Options {
    port: u16,
    time_scale: f64,
    profile: Profile,
    gates: usize,
    access: Option<AccessConfig>,
}

impl Options {
    fn machine_profile(&self) -> MachineProfile {
        match self.profile {
            Profile::Basic => MachineProfile::basic(),
            Profile::Voron => MachineProfile::voron(),
            Profile::HappyHare => MachineProfile::happy_hare(self.gates),
            Profile::Ercf => MachineProfile::ercf(self.gates),
            Profile::Toolchanger => MachineProfile::toolchanger(),
            Profile::ToolchangerRequired => MachineProfile::toolchanger_required(),
            Profile::HappyHareUnits => MachineProfile::happy_hare_units(),
            Profile::ToolchangerMmu => MachineProfile::toolchanger_mmu(),
            Profile::Afc => MachineProfile::afc(),
            Profile::AfcToolchanger => MachineProfile::afc_toolchanger(),
        }
    }
}

fn parse_options(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        port: DEFAULT_PORT,
        time_scale: DEFAULT_TIME_SCALE,
        profile: Profile::Voron,
        gates: DEFAULT_MMU_GATES,
        access: None,
    };

    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--port" => options.port = value.parse().map_err(|_| format!("invalid port '{value}'"))?,
            "--time-scale" => options.time_scale = parse_time_scale(&value)?,
            "--profile" => options.profile = parse_profile(&value)?,
            "--gates" => options.gates = parse_gates(&value)?,
            "--auth" => options.access = Some(parse_access(&value)?),
            _ => return Err(format!("unknown option '{flag}'")),
        }
    }
    Ok(options)
}

fn parse_access(value: &str) -> Result<AccessConfig, String> {
    match value.split_once(':') {
        Some((username, password)) if !username.is_empty() => Ok(AccessConfig::new(username, password)),
        _ => Err(format!("invalid login '{value}', expected <user>:<password>")),
    }
}

fn parse_profile(value: &str) -> Result<Profile, String> {
    match value {
        "basic" => Ok(Profile::Basic),
        "voron" => Ok(Profile::Voron),
        "happy-hare" => Ok(Profile::HappyHare),
        "ercf" => Ok(Profile::Ercf),
        "toolchanger" => Ok(Profile::Toolchanger),
        "toolchanger-required" => Ok(Profile::ToolchangerRequired),
        "happy-hare-units" => Ok(Profile::HappyHareUnits),
        "toolchanger-mmu" => Ok(Profile::ToolchangerMmu),
        "afc" => Ok(Profile::Afc),
        "afc-toolchanger" => Ok(Profile::AfcToolchanger),
        _ => Err(format!("unknown profile '{value}'")),
    }
}

fn parse_gates(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|gates| (1..=MachineProfile::MAX_MMU_GATES).contains(gates))
        .ok_or_else(|| format!("invalid gate count '{value}'"))
}

fn parse_time_scale(value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .ok_or_else(|| format!("invalid time scale '{value}'"))
}

async fn run() -> Result<(), String> {
    let options = parse_options(std::env::args().skip(1))?;
    let listener = TcpListener::bind(("127.0.0.1", options.port))
        .await
        .map_err(|error| format!("cannot listen on port {}: {error}", options.port))?;

    println!("Fake-Moonraker listening. Start freakymate with:");
    println!("  cargo run -p freakymate -- --moonraker http://127.0.0.1:{}", options.port);
    if let Some(access) = &options.access {
        println!("Login required: user '{}', API key '{}'", access.username, access.api_key);
    }

    let config = ServerConfig {
        time_scale: options.time_scale,
        status_interval: Duration::from_millis(250),
        profile: options.machine_profile(),
        access: options.access.clone(),
    };
    serve(listener, config).await.map_err(|error| error.to_string())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
