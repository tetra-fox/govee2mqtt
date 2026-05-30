use clap::Parser;
use govee_api::lan_api::LanDiscoArguments;
use govee_api::platform_api::GoveeApiArguments;
use govee_api::undoc_api::UndocApiArguments;

mod commands;
mod hass_mqtt;
mod service;
mod version_info;

use crate::service::hass::HassArguments;

#[derive(clap::Parser, Debug)]
#[command(version = version_info::govee_version(),  propagate_version=true)]
pub struct Args {
    #[command(flatten)]
    api_args: GoveeApiArguments,
    #[command(flatten)]
    lan_disco_args: LanDiscoArguments,
    #[command(flatten)]
    undoc_args: UndocApiArguments,
    #[command(flatten)]
    hass_args: HassArguments,

    #[command(subcommand)]
    cmd: SubCommand,
}

#[derive(clap::Parser, Debug)]
pub enum SubCommand {
    LanControl(commands::lan_control::LanControlCommand),
    LanDisco(commands::lan_disco::LanDiscoCommand),
    ListHttp(commands::list_http::ListHttpCommand),
    List(commands::list::ListCommand),
    HttpControl(commands::http_control::HttpControlCommand),
    Serve(commands::serve::ServeCommand),
    Undoc(commands::undoc::UndocCommand),
    BleProbe(commands::ble_probe::BleProbeCommand),
    BleControl(commands::ble_control::BleControlCommand),
}

impl Args {
    pub async fn run(&self) -> anyhow::Result<()> {
        match &self.cmd {
            SubCommand::LanControl(cmd) => cmd.run(self).await,
            SubCommand::LanDisco(cmd) => cmd.run(self).await,
            SubCommand::ListHttp(cmd) => cmd.run(self).await,
            SubCommand::HttpControl(cmd) => cmd.run(self).await,
            SubCommand::List(cmd) => cmd.run(self).await,
            SubCommand::Serve(cmd) => cmd.run(self).await,
            SubCommand::Undoc(cmd) => cmd.run(self).await,
            SubCommand::BleProbe(cmd) => cmd.run(self).await,
            SubCommand::BleControl(cmd) => cmd.run(self).await,
        }
    }
}

/// The daemon's local timezone: `$TZ` if set, else the system zone, else UTC.
/// Used for log timestamps and for the BLE SYNC_TIME frame, so device timers
/// fire against the same wall clock the daemon reports.
pub fn resolve_timezone() -> chrono_tz::Tz {
    std::env::var("TZ")
        .or_else(|_| iana_time_zone::get_timezone())
        .ok()
        .and_then(|name| name.parse().ok())
        .unwrap_or(chrono_tz::UTC)
}

fn setup_logger() {
    let tz = resolve_timezone();
    let utc_suffix = if tz == chrono_tz::UTC { "Z" } else { "" };

    env_logger::builder()
        // A bit of boilerplate here to get timestamps printed in local time.
        // <https://github.com/rust-cli/env_logger/issues/158>
        .format(move |buf, record| {
            use chrono::Utc;
            use std::io::Write;

            let level_style = buf.default_level_style(record.level());
            write!(
                buf,
                "[{}{utc_suffix} ",
                Utc::now().with_timezone(&tz).format("%Y-%m-%dT%H:%M:%S")
            )?;
            write!(buf, "{level_style}{:<5}{level_style:#}", record.level())?;
            if let Some(path) = record.module_path() {
                write!(buf, " {}", path)?;
            }
            writeln!(buf, "] {}", record.args())
        })
        .filter_level(log::LevelFilter::Info)
        .parse_env("RUST_LOG")
        .init();
}

#[tokio::main(worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    color_backtrace::install();
    if let Ok(path) = dotenvy::dotenv() {
        eprintln!("Loading environment overrides from {path:?}");
    }

    setup_logger();

    let args = Args::parse();
    args.run().await
}
