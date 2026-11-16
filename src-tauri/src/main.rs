mod app;
mod config;
mod ctl;
mod engine;
mod events;
mod headless;
mod heuristics;
mod opencode;
mod screen;
mod translate;
mod usage;
mod watchdog;
use anyhow::{bail, Context, Result};
use std::process::ExitCode;
fn logging() -> Result<tracing_appender::non_blocking::WorkerGuard> {
    let dir = dirs::state_dir()
        .context("No se encuentra el directorio de estado")?
        .join("quickask");
    std::fs::create_dir_all(&dir)?;
    let writer = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("quickask")
        .filename_suffix("log")
        .max_log_files(7)
        .latest_symlink("quickask.log")
        .build(&dir)?;
    let (w, g) = tracing_appender::non_blocking(writer);
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "quickask=info".into()),
        )
        .with_writer(w)
        .init();
    Ok(g)
}
fn run() -> Result<bool> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("quickask {}", env!("CARGO_PKG_VERSION"));
        return Ok(true);
    }
    if let Some(i) = args.iter().position(|a| a == "--ask") {
        let text = args
            .get(i + 1)
            .filter(|s| !s.starts_with("--"))
            .context("Falta el texto de --ask")?
            .clone();
        let _guard = logging()?;
        screen::cleanup()?;
        let rt = tokio::runtime::Runtime::new()?;
        return rt.block_on(headless::run(text, args.iter().any(|a| a == "--screen")));
    }
    if args
        .iter()
        .any(|a| !["--toggle", "--background", "--quit"].contains(&a.as_str()))
    {
        bail!(
            "Opciones: --toggle, --background, --quit, --ask TEXTO [--screen] [--new], --version"
        );
    }
    let path = ctl::runtime_dir()?.join("ctl.sock");
    let command = if args.iter().any(|a| a == "--quit") {
        "quit"
    } else if args.iter().any(|a| a == "--background") {
        "hide"
    } else {
        "toggle"
    };
    let cmd = ctl::Command {
        cmd: command.into(),
        activation_token: std::env::var("XDG_ACTIVATION_TOKEN")
            .or_else(|_| std::env::var("DESKTOP_STARTUP_ID"))
            .ok(),
    };
    if ctl::send(&path, &cmd).is_ok() {
        return Ok(true);
    }
    if command == "quit" {
        return Ok(true);
    }
    let signals =
        app::shutdown_signals().context("No se pudieron registrar las señales de salida")?;
    let listener = ctl::bind(&path)?;
    let socket_guard = std::sync::Arc::new(ctl::SocketGuard::new(&path)?);
    let _guard = logging()?;
    screen::cleanup()?;
    let result = app::run(
        config::Config::load()?,
        listener,
        !args.iter().any(|a| a == "--background"),
        socket_guard,
        signals,
    );
    screen::shutdown();
    result?;
    Ok(true)
}
fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("quickask: {e:#}");
            ExitCode::FAILURE
        }
    }
}
