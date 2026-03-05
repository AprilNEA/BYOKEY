use anyhow::Result;

use crate::DaemonArgs;

fn start_opts(args: DaemonArgs) -> byokey_daemon::process::StartOptions {
    byokey_daemon::process::StartOptions {
        exe: None,
        config: args.server.config,
        port: args.server.port,
        host: args.server.host,
        db: args.server.db,
        log_file: args.log_file,
        pid_file: None,
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
    println!("byokey stopped (pid {})", result.pid);
    Ok(())
}

pub fn cmd_restart(args: DaemonArgs) -> Result<()> {
    let result = byokey_daemon::process::restart(start_opts(args))?;
    println!("byokey started (pid {})", result.pid);
    println!("logs: {}", result.log_path.display());
    Ok(())
}

// ── Autostart ────────────────────────────────────────────────────────────────

pub fn cmd_autostart_enable(args: DaemonArgs) -> Result<()> {
    let opts = byokey_daemon::autostart::AutostartOptions {
        exe: None,
        config: args.server.config,
        port: args.server.port,
        host: args.server.host,
        db: args.server.db,
        log_file: args.log_file,
    };
    let result = byokey_daemon::autostart::enable(opts)?;
    println!("autostart enabled ({})", result.backend);
    println!("service file: {}", result.service_file.display());
    Ok(())
}

pub fn cmd_autostart_disable() -> Result<()> {
    byokey_daemon::autostart::disable()?;
    println!("autostart disabled");
    Ok(())
}

pub fn cmd_autostart_status() -> Result<()> {
    let st = byokey_daemon::autostart::status()?;
    if !st.enabled {
        println!("autostart: disabled");
        return Ok(());
    }
    println!("autostart: enabled");
    if let Some(ref path) = st.service_file {
        println!("service:   {}", path.display());
    }
    println!(
        "running:   {}",
        if st.service_running { "yes" } else { "no" }
    );
    Ok(())
}
