use anyhow::Result;

use crate::DaemonArgs;

fn start_opts(args: DaemonArgs) -> byokey_daemon::process::StartOptions {
    byokey_daemon::process::StartOptions {
        exe: None,
        config: args.server.config,
        port: args.server.port,
        host: args.server.host,
        db: args.server.db,
        log_file: args.server.log_file,
        pid_file: None,
    }
}

fn service_opts(args: DaemonArgs) -> byokey_daemon::service::ServiceOptions {
    byokey_daemon::service::ServiceOptions {
        exe: None,
        config: args.server.config,
        port: args.server.port,
        host: args.server.host,
        db: args.server.db,
        log_file: args.server.log_file,
    }
}

pub fn cmd_start(args: DaemonArgs) -> Result<()> {
    let result = byokey_daemon::process::start(start_opts(args))?;
    println!("byokey started (pid {})", result.pid);
    println!("logs: {}", result.log_path.display());
    Ok(())
}

pub fn cmd_stop() -> Result<()> {
    let result = byokey_daemon::process::stop()?;
    match result.pid {
        Some(pid) => println!("byokey stopped (pid {pid})"),
        None => println!("byokey stopped"),
    }
    Ok(())
}

pub fn cmd_restart(args: DaemonArgs) -> Result<()> {
    let result = byokey_daemon::process::restart(start_opts(args))?;
    println!("byokey started (pid {})", result.pid);
    println!("logs: {}", result.log_path.display());
    Ok(())
}

pub fn cmd_reload() -> Result<()> {
    byokey_daemon::control::reload()?;
    println!("configuration reloaded");
    Ok(())
}

// ── Service (OS-managed) ─────────────────────────────────────────────────────

pub fn cmd_service_install(args: DaemonArgs) -> Result<()> {
    let result = byokey_daemon::service::install(service_opts(args))?;
    println!("service installed ({})", result.backend);
    println!("label:   {}", result.label);
    // Installing via `service-manager` sets autostart=true but does not start
    // the service immediately on all backends. Start it now for convenience.
    byokey_daemon::service::start()?;
    println!("service started");
    Ok(())
}

pub fn cmd_service_uninstall() -> Result<()> {
    byokey_daemon::service::uninstall()?;
    println!("service uninstalled");
    Ok(())
}

pub fn cmd_service_start() -> Result<()> {
    byokey_daemon::service::start()?;
    println!("service started");
    Ok(())
}

pub fn cmd_service_stop() -> Result<()> {
    byokey_daemon::service::stop()?;
    println!("service stopped");
    Ok(())
}

pub fn cmd_service_status() -> Result<()> {
    let st = byokey_daemon::service::status()?;
    println!("backend:   {}", st.backend);
    println!("installed: {}", if st.installed { "yes" } else { "no" });
    println!("running:   {}", if st.running { "yes" } else { "no" });
    Ok(())
}
