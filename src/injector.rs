use crate::config::InjectMethod;
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
    // wtype uses the Wayland virtual-keyboard protocol; no daemon needed.
    // Pass text as a single argument — Command::arg() doesn't invoke a shell,
    // so no escaping is needed.
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
    // ydotool requires the ydotoold daemon. If YDOTOOL_SOCKET is not set,
    // ydotool looks for /tmp/.ydotool_socket by default.
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

fn which_in_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .any(|p| p.join(name).is_file())
        })
        .unwrap_or(false)
}
