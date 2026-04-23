use clap::{Args, Parser, Subcommand};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;

use anyclaude::config::Config;

// ── CLI 结构 ──────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "anyclaude", version)]
#[command(about = "TUI wrapper for Claude Code with multi-backend support")]
#[command(args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[command(flatten)]
    run: RunArgs,
}

/// Default run args (used when no subcommand is given).
#[derive(Args, Default)]
struct RunArgs {
    /// Override default backend (see config for available backends)
    #[arg(long, value_name = "NAME")]
    backend: Option<String>,

    /// Arguments passed to claude
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Show running status of anyclaude
    Status,

    /// View debug logs
    Logs {
        /// Number of recent lines to show
        #[arg(short = 'n', long, default_value = "50")]
        lines: usize,
        /// Follow log output (Ctrl+C to exit, like tail -f)
        #[arg(short, long)]
        follow: bool,
    },

    /// Stop a running anyclaude instance
    Stop,

    /// Uninstall anyclaude
    Uninstall {
        /// Also remove configuration directory (~/.config/anyclaude/)
        #[arg(long)]
        purge: bool,
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
}

// ── PID 文件工具 ──────────────────────────────────────────────────────────────

fn pid_file_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".config/anyclaude/anyclaude.pid")
}

fn read_pid() -> Option<u32> {
    std::fs::read_to_string(pid_file_path())
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

fn is_process_running(pid: u32) -> bool {
    // send signal 0: checks process existence without actually signalling
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// RAII guard that removes the PID file on drop.
struct PidGuard(PathBuf);
impl Drop for PidGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

// ── 子命令实现 ────────────────────────────────────────────────────────────────

fn cmd_status() -> io::Result<()> {
    match read_pid() {
        Some(pid) if is_process_running(pid) => {
            println!("● anyclaude 正在运行  (PID: {})", pid);
            if let Ok(cfg) = Config::load() {
                println!("  代理地址:  http://{}", cfg.proxy.bind_addr);
                println!("  WebUI地址: http://{}", cfg.webui.bind_addr);
                if cfg.webui.username.is_some() {
                    println!("  WebUI认证: 已启用（需要账号密码）");
                }
            }
        }
        Some(_) => {
            println!("○ anyclaude 未运行（存在过期 PID 文件，将清理）");
            let _ = std::fs::remove_file(pid_file_path());
        }
        None => {
            println!("○ anyclaude 未运行");
        }
    }
    Ok(())
}

fn cmd_logs(lines: usize, follow: bool) -> io::Result<()> {
    let raw_path = Config::load()
        .ok()
        .map(|c| c.debug_logging.file_path.clone())
        .unwrap_or_else(|| "~/.config/anyclaude/logs/debug.log".to_string());

    let log_path = if raw_path.starts_with("~/") {
        dirs::home_dir()
            .map(|h| h.join(&raw_path[2..]))
            .unwrap_or_else(|| PathBuf::from(&raw_path))
    } else {
        PathBuf::from(&raw_path)
    };

    if !log_path.exists() {
        eprintln!("日志文件不存在: {}", log_path.display());
        eprintln!("请先在配置中启用日志: [debug_logging] level = \"verbose\"");
        std::process::exit(1);
    }

    let mut cmd = std::process::Command::new("tail");
    cmd.arg("-n").arg(lines.to_string());
    if follow {
        cmd.arg("-f");
    }
    cmd.arg(&log_path);
    let status = cmd.status()?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

fn cmd_stop() -> io::Result<()> {
    match read_pid() {
        Some(pid) if is_process_running(pid) => {
            let ret = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            if ret == 0 {
                println!("已向 anyclaude (PID: {}) 发送停止信号", pid);
            } else {
                eprintln!("发送停止信号失败，请手动执行: kill {}", pid);
                std::process::exit(1);
            }
        }
        Some(_) => {
            println!("anyclaude 未运行，清理过期 PID 文件");
            let _ = std::fs::remove_file(pid_file_path());
        }
        None => {
            println!("anyclaude 未运行");
        }
    }
    Ok(())
}

fn cmd_uninstall(purge: bool, yes: bool) -> io::Result<()> {
    if !yes {
        let suffix = if purge { "（含配置文件）" } else { "" };
        print!("确认卸载 anyclaude{}？[y/N]: ", suffix);
        io::stdout().flush()?;
        let mut buf = [0u8; 4];
        let n = io::stdin().read(&mut buf).unwrap_or(0);
        let input = std::str::from_utf8(&buf[..n]).unwrap_or("").trim().to_lowercase();
        if input != "y" {
            println!("已取消");
            return Ok(());
        }
    }

    let binary_path = std::env::current_exe()?;
    if binary_path.exists() {
        std::fs::remove_file(&binary_path)?;
        println!("✓ 已删除二进制: {}", binary_path.display());
    }

    let pid_path = pid_file_path();
    if pid_path.exists() {
        let _ = std::fs::remove_file(&pid_path);
    }

    if purge {
        let config_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".config/anyclaude");
        if config_dir.exists() {
            std::fs::remove_dir_all(&config_dir)?;
            println!("✓ 已删除配置目录: {}", config_dir.display());
        }
    } else {
        println!("  配置已保留: ~/.config/anyclaude/");
        println!("  完全删除请运行: anyclaude uninstall --purge");
    }

    println!("卸载完成");
    Ok(())
}

// ── 入口 ──────────────────────────────────────────────────────────────────────

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    // Subcommands don't need a TTY — handle them first
    if let Some(cmd) = cli.command {
        return match cmd {
            Commands::Status => cmd_status(),
            Commands::Logs { lines, follow } => cmd_logs(lines, follow),
            Commands::Stop => cmd_stop(),
            Commands::Uninstall { purge, yes } => cmd_uninstall(purge, yes),
        };
    }

    // Default: start TUI — enter raw mode IMMEDIATELY to capture early tmux input.
    let is_tty = io::stdin().is_terminal();
    if is_tty {
        enable_raw_mode()?;
    }

    let result = run_main(cli.run);

    if is_tty && result.is_err() {
        let _ = disable_raw_mode();
    }

    result
}

fn run_main(run: RunArgs) -> io::Result<()> {
    // Load config — fail fast on invalid config
    let config = match Config::load() {
        Ok(config) => config,
        Err(e) => {
            let _ = disable_raw_mode();
            eprintln!("Error: Failed to load config: {}", e);
            eprintln!("Config file: {}", Config::config_path().display());
            std::process::exit(1);
        }
    };

    if let Some(ref backend_name) = run.backend {
        let exists = config.backends.iter().any(|b| &b.name == backend_name);
        if !exists {
            let _ = disable_raw_mode();
            let available: Vec<_> = config.backends.iter().map(|b| b.name.as_str()).collect();
            eprintln!("Error: Backend '{}' not found in config", backend_name);
            if available.is_empty() {
                eprintln!("No backends configured");
            } else {
                eprintln!("Available backends: {}", available.join(", "));
            }
            std::process::exit(1);
        }
    }

    // Write PID file so `anyclaude status` / `stop` can find this process.
    let pid_path = pid_file_path();
    if let Some(parent) = pid_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&pid_path, std::process::id().to_string());
    let _pid_guard = PidGuard(pid_path); // auto-deleted on exit

    anyclaude::ui::run(run.backend, run.args)
}
