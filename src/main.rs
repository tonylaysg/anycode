use clap::{Args, Parser, Subcommand};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;

use anyclaude::config::{Config, save_config};

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

    /// Start the WebUI configuration server (without launching Claude Code)
    Webui {
        /// Override bind address (e.g. 0.0.0.0:47191 for LAN access)
        #[arg(long, value_name = "ADDR")]
        bind: Option<String>,
        /// Run as background daemon (detach from terminal, write PID file)
        #[arg(long, short = 'd')]
        daemon: bool,
        /// Stop the running WebUI daemon
        #[arg(long)]
        stop: bool,
    },

    /// Change WebUI access mode (local / lan / public / custom address)
    Bind {
        /// Access mode: local, lan, public, or a custom address like 0.0.0.0:8080
        #[arg(value_name = "MODE|ADDR")]
        mode: String,
    },

    /// Set or reset WebUI login credentials
    Passwd,

    /// Reset Claude Code auth state (clears cached env from previous sessions)
    Reset {
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
}

// ── PID 文件工具 ──────────────────────────────────────────────────────────────

fn webui_pid_file_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".config/anyclaude/webui.pid")
}

fn read_webui_pid() -> Option<u32> {
    std::fs::read_to_string(webui_pid_file_path())
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

fn is_process_running(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{}", pid)).exists()
    }
    #[cfg(not(target_os = "linux"))]
    unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0
    }
}

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

/// Returns true only when PID exists AND its /proc/PID/comm is "anyclaude".
/// This prevents false-positives when the OS reuses a stale PID.
fn is_anyclaude_running(pid: u32) -> bool {
    // Primary: check /proc/<pid>/comm (Linux-only, fast)
    #[cfg(target_os = "linux")]
    {
        if let Ok(comm) = std::fs::read_to_string(format!("/proc/{}/comm", pid)) {
            return comm.trim() == "anyclaude";
        }
        return false;
    }
    // Fallback for non-Linux: signal-0 only (no process-identity check)
    #[cfg(not(target_os = "linux"))]
    unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0
    }
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
    // ── 主进程 (TUI + 代理) ──────────────────────────────────────────────────
    match read_pid() {
        Some(pid) if is_anyclaude_running(pid) => {
            println!("● anyclaude 主进程  正在运行  (PID: {})", pid);
            if let Ok(cfg) = Config::load() {
                println!("  代理地址:  http://{}", cfg.proxy.bind_addr);
            }
        }
        Some(_) => {
            println!("○ anyclaude 主进程  未运行（清理过期 PID）");
            let _ = std::fs::remove_file(pid_file_path());
        }
        None => {
            println!("○ anyclaude 主进程  未运行");
        }
    }

    // ── WebUI 守护进程 ────────────────────────────────────────────────────────
    match read_webui_pid() {
        Some(pid) if is_process_running(pid) => {
            println!("● WebUI 守护进程  正在运行  (PID: {})", pid);
            if let Ok(cfg) = Config::load() {
                println!("  WebUI地址: http://{}", cfg.webui.bind_addr);
                if cfg.webui.username.is_some() {
                    println!("  WebUI认证: 已启用（需要账号密码）");
                }
            }
        }
        Some(_) => {
            println!("○ WebUI 守护进程  未运行（清理过期 PID）");
            let _ = std::fs::remove_file(webui_pid_file_path());
        }
        None => {
            println!("○ WebUI 守护进程  未运行  (使用 'anyclaude webui --daemon' 启动)");
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
        Some(pid) if is_anyclaude_running(pid) => {
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

fn cmd_reset(yes: bool) -> io::Result<()> {
    if !yes {
        println!("此命令将清理 AnyClaude 注入到 Claude Code 的残留状态：");
        println!("  - 停止运行中的 anyclaude 实例");
        println!("  - 清理 ~/.claude/session-env/ 缓存的环境变量");
        println!("  - 清理 ~/.claude/sessions/ 中的会话记录");
        println!();
        print!("确认执行？[y/N]: ");
        io::stdout().flush()?;
        let mut buf = [0u8; 4];
        let n = io::stdin().read(&mut buf).unwrap_or(0);
        let input = std::str::from_utf8(&buf[..n]).unwrap_or("").trim().to_lowercase();
        if input != "y" {
            println!("已取消");
            return Ok(());
        }
    }

    // 停止运行中的 anyclaude 实例
    if let Some(pid) = read_pid() {
        if is_anyclaude_running(pid) {
            let ret = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            if ret == 0 {
                println!("✓ 已停止 anyclaude (PID: {})", pid);
                let _ = std::fs::remove_file(pid_file_path());
            }
        }
    }

    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    let claude_dir = home.join(".claude");

    // 清理 session-env（anyclaude 注入的 ANTHROPIC_BASE_URL 等可能被 Claude Code 缓存于此）
    let session_env_dir = claude_dir.join("session-env");
    if session_env_dir.exists() {
        match std::fs::remove_dir_all(&session_env_dir) {
            Ok(_) => println!("✓ 已清理 ~/.claude/session-env/"),
            Err(e) => eprintln!("  清理 session-env 失败: {}", e),
        }
    } else {
        println!("  ~/.claude/session-env/ 不存在，跳过");
    }

    // 清理 sessions（旧会话可能携带 anyclaude-proxy 凭证缓存）
    let sessions_dir = claude_dir.join("sessions");
    if sessions_dir.exists() {
        match std::fs::remove_dir_all(&sessions_dir) {
            Ok(_) => println!("✓ 已清理 ~/.claude/sessions/"),
            Err(e) => eprintln!("  清理 sessions 失败: {}", e),
        }
    } else {
        println!("  ~/.claude/sessions/ 不存在，跳过");
    }

    println!();
    println!("重置完成。请重新运行 anyclaude，Claude Code 将重新进行登录认证。");
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

fn cmd_webui_stop() -> io::Result<()> {
    match read_webui_pid() {
        Some(pid) if is_process_running(pid) => {
            let ret = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            if ret == 0 {
                println!("已向 WebUI 守护进程 (PID: {}) 发送停止信号", pid);
                let _ = std::fs::remove_file(webui_pid_file_path());
            } else {
                eprintln!("发送停止信号失败，请手动执行: kill {}", pid);
                std::process::exit(1);
            }
        }
        Some(_) => {
            println!("WebUI 守护进程未运行，清理过期 PID 文件");
            let _ = std::fs::remove_file(webui_pid_file_path());
        }
        None => {
            println!("WebUI 守护进程未运行");
        }
    }
    Ok(())
}

fn cmd_webui(bind_override: Option<String>, daemon: bool) -> io::Result<()> {
    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: Failed to load config: {}", e);
            eprintln!("Config file: {}", Config::config_path().display());
            std::process::exit(1);
        }
    };

    let bind_addr = bind_override
        .clone()
        .unwrap_or_else(|| config.webui.bind_addr.clone());
    let username = config.webui.username.clone();
    let password = config.webui.password.clone();

    // ── Daemon mode: re-exec self without --daemon, detached ──────────────────
    if daemon {
        let log_dir = Config::config_path()
            .parent()
            .unwrap_or_else(|| std::path::Path::new("/tmp"))
            .to_path_buf();
        let log_path = log_dir.join("webui.log");
        let pid_path = log_dir.join("webui.pid");

        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;

        let mut cmd = std::process::Command::new(std::env::current_exe()?);
        cmd.arg("webui");
        if let Some(ref b) = bind_override {
            cmd.args(["--bind", b]);
        }
        let child = cmd
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(log_file.try_clone()?))
            .stderr(std::process::Stdio::from(log_file))
            .spawn()?;

        let pid = child.id();
        std::fs::write(&pid_path, pid.to_string())?;

        let auth_note = if username.is_some() && password.is_some() {
            " (账号密码保护)"
        } else {
            " (无需认证)"
        };
        println!("WebUI 已在后台启动 (PID {})", pid);
        println!("地址: http://{}{}", bind_addr, auth_note);
        println!("日志: {}", log_path.display());
        println!("停止: anyclaude webui --stop  或  kill {}", pid);
        return Ok(());
    }

    // ── Foreground mode ───────────────────────────────────────────────────────
    let auth_note = if username.is_some() && password.is_some() {
        " (账号密码保护)"
    } else {
        " (无需认证)"
    };

    let config_path = Config::config_path();
    let config_store = anyclaude::config::ConfigStore::new(config.clone(), config_path);
    let backend_state = anyclaude::backend::BackendState::from_config(config)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    let webui_state = anyclaude::proxy::webui::WebuiState {
        config_store,
        backend_state,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async move {
        match anyclaude::proxy::webui::bind_webui(&bind_addr).await {
            Ok((addr, listener)) => {
                println!("WebUI 已启动: http://{}{}", addr, auth_note);
                println!("按 Ctrl+C 停止");
                if let Err(e) = anyclaude::proxy::webui::serve_webui(listener, webui_state, username, password).await {
                    eprintln!("WebUI 错误: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Error: 无法绑定 {}: {}", bind_addr, e);
                std::process::exit(1);
            }
        }
    });

    Ok(())
}

fn load_config_or_exit() -> Config {
    match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: Failed to load config: {}", e);
            eprintln!("Config file: {}", Config::config_path().display());
            std::process::exit(1);
        }
    }
}

fn cmd_bind(mode: &str) -> io::Result<()> {
    let bind_addr = match mode {
        "local" | "localhost" => "127.0.0.1:47191".to_string(),
        "lan" | "public" => "0.0.0.0:47191".to_string(),
        addr if addr.contains(':') => addr.to_string(),
        _ => {
            eprintln!("Error: 无效模式 '{}'", mode);
            eprintln!("可选值: local / lan / public / 自定义地址(如 0.0.0.0:9000)");
            std::process::exit(1);
        }
    };

    let mut config = load_config_or_exit();
    let old = config.webui.bind_addr.clone();
    config.webui.bind_addr = bind_addr.clone();

    save_config(&Config::config_path(), &config)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    println!("WebUI 绑定地址已更新");
    println!("  旧: {}", old);
    println!("  新: {}", bind_addr);
    if bind_addr.starts_with("0.0.0.0") && config.webui.password.is_none() {
        println!();
        println!("警告: 已开放外部访问，但未设置登录密码！");
        println!("建议运行: anyclaude passwd");
    }
    println!();
    println!("重启 WebUI 后生效: anyclaude webui");
    Ok(())
}

fn cmd_passwd() -> io::Result<()> {
    let mut config = load_config_or_exit();

    println!("=== 设置 WebUI 登录账号密码 ===");
    println!("（直接回车保留现有值，输入 '-' 清除密码启用免登录）");
    println!();

    // Username
    let cur_user = config.webui.username.as_deref().unwrap_or("（未设置）");
    print!("用户名 [当前: {}]: ", cur_user);
    io::stdout().flush()?;
    let mut new_user = String::new();
    io::stdin().read_line(&mut new_user)?;
    let new_user = new_user.trim();

    // Password
    let new_pass = read_secret("密码 [回车保留 / 输入 '-' 清除]: ")?;

    // Apply changes
    match new_user {
        "" => {}                                          // keep existing
        "-" => config.webui.username = None,
        u  => config.webui.username = Some(u.to_string()),
    }
    match new_pass.as_str() {
        "" => {}                                          // keep existing
        "-" => { config.webui.password = None; config.webui.username = None; }
        p  => config.webui.password = Some(p.to_string()),
    }

    // Ensure username is set when password is set
    if config.webui.password.is_some() && config.webui.username.is_none() {
        config.webui.username = Some("admin".to_string());
        println!("用户名未设置，已自动设为 admin");
    }

    save_config(&Config::config_path(), &config)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    if config.webui.password.is_some() {
        println!("✓ 登录密码已设置（用户名: {}）", config.webui.username.as_deref().unwrap_or("admin"));
    } else {
        println!("✓ 已清除密码，WebUI 无需登录即可访问");
    }
    println!("重启 WebUI 后生效: anyclaude webui");
    Ok(())
}

/// Read a line from stdin with echo disabled (cross-platform, no external crates).
fn read_secret(prompt: &str) -> io::Result<String> {
    print!("{}", prompt);
    io::stdout().flush()?;

    // Save terminal state, disable echo, read, restore
    let saved = std::process::Command::new("stty")
        .arg("-g")
        .stdin(std::fs::File::open("/dev/tty")?)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let _ = std::process::Command::new("stty")
        .arg("-echo")
        .stdin(std::fs::File::open("/dev/tty")?)
        .status();

    let mut val = String::new();
    let result = io::stdin().read_line(&mut val);

    // Restore terminal (always, even on error)
    if !saved.is_empty() {
        let _ = std::process::Command::new("stty")
            .arg(&saved)
            .stdin(std::fs::File::open("/dev/tty").unwrap_or_else(|_| unsafe {
                use std::os::unix::io::FromRawFd;
                std::fs::File::from_raw_fd(0)
            }))
            .status();
    } else {
        let _ = std::process::Command::new("stty")
            .arg("echo")
            .stdin(std::fs::File::open("/dev/tty").unwrap_or_else(|_| unsafe {
                use std::os::unix::io::FromRawFd;
                std::fs::File::from_raw_fd(0)
            }))
            .status();
    }
    println!(); // newline after hidden input

    result?;
    Ok(val.trim().to_string())
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
            Commands::Webui { bind, daemon, stop } => {
                if stop {
                    return cmd_webui_stop();
                }
                cmd_webui(bind, daemon)
            }
            Commands::Bind { mode } => cmd_bind(&mode),
            Commands::Passwd => cmd_passwd(),
            Commands::Reset { yes } => cmd_reset(yes),
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
