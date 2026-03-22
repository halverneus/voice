use crate::config::InjectMethod;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// Inject `text` into the currently focused window using the configured method.
pub fn inject_text(text: &str, method: &InjectMethod, delay_ms: u64) {
    if delay_ms > 0 {
        std::thread::sleep(Duration::from_millis(delay_ms));
    }

    // Append a trailing space so consecutive segments don't run together.
    let to_type = format!("{} ", text);

    match method {
        InjectMethod::Wtype => inject_wtype(&to_type),
        InjectMethod::Ydotool => inject_ydotool(&to_type),
    }
}

fn inject_wtype(text: &str) {
    match Command::new("wtype").arg(text).output() {
        Ok(out) if !out.status.success() => {
            let err = String::from_utf8_lossy(&out.stderr);
            log::warn!("wtype failed: {}", err);
        }
        Err(e) => log::error!("wtype not found or failed to run: {}", e),
        _ => {}
    }
}

fn inject_ydotool(text: &str) {
    match Command::new("ydotool")
        .args(["type", "--key-delay", "0", "--", text])
        .output()
    {
        Ok(out) if !out.status.success() => {
            let err = String::from_utf8_lossy(&out.stderr);
            log::warn!("ydotool failed: {}", err);
        }
        Err(e) => log::error!("ydotool not found or failed to run: {}", e),
        _ => {}
    }
}

/// Quick sanity check: can we find the requested injection tool?
pub fn check_inject_tool(method: &InjectMethod) -> bool {
    let tool = match method {
        InjectMethod::Wtype => "wtype",
        InjectMethod::Ydotool => "ydotool",
    };
    which_in_path(tool)
}

/// Ensure `ydotoold` is running.
///
/// Checks whether the ydotoold socket already exists. If it doesn't, spawns
/// `ydotoold` in the background and waits up to ~500 ms for the socket to
/// appear. Safe to call multiple times — does nothing if the daemon is
/// already up.
pub fn ensure_ydotoold_running() {
    let socket = ydotool_socket_path();

    if socket.exists() {
        log::debug!("ydotoold already running (socket {})", socket.display());
        return;
    }

    log::info!("ydotoold socket not found — starting ydotoold in background");

    // Spawn detached. Dropping the Child handle on Linux does NOT kill the
    // child process; it lives on as a daemon.
    match Command::new("ydotoold").spawn() {
        Ok(_child) => {
            // Wait for the socket to appear (up to 500 ms, checking every 50 ms)
            for _ in 0..10 {
                std::thread::sleep(Duration::from_millis(50));
                if socket.exists() {
                    log::info!("ydotoold started successfully");
                    return;
                }
            }
            log::warn!(
                "ydotoold spawned but socket {} not found after 500 ms — \
                 it may need uinput permission (add user to `input` group)",
                socket.display()
            );
        }
        Err(e) => log::error!("Failed to start ydotoold: {}", e),
    }
}

/// Returns the ydotoold socket path: $YDOTOOL_SOCKET or /tmp/.ydotool_socket.
fn ydotool_socket_path() -> PathBuf {
    std::env::var_os("YDOTOOL_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/.ydotool_socket"))
}

fn which_in_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|p| p.join(name).is_file()))
        .unwrap_or(false)
}
