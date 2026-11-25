//! Abstract DevTools socket discovery from `/proc/net/unix`.

use std::collections::BTreeMap;

use regex::Regex;

/// Discovered Android WebView/Chrome DevTools abstract Unix socket.
#[derive(Debug, Clone)]
pub(crate) struct DevToolsSocket {
    pub(crate) socket: String,
    pub(crate) pid: Option<u32>,
    pub(crate) process: String,
}

fn read_cmdline(pid: u32) -> String {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|b| {
            String::from_utf8_lossy(&b)
                .replace('\0', " ")
                .trim()
                .to_owned()
        })
        .unwrap_or_default()
}

/// Parses `/proc/net/unix` content for `@webview_devtools_remote*` / `@chrome_devtools_remote*`.
pub(crate) fn parse_devtools_sockets(proc_net_unix: &str) -> Vec<DevToolsSocket> {
    let Ok(re) = Regex::new(r"@((?:webview|chrome)_devtools_remote(?:_(\d+))?)") else {
        return Vec::new();
    };
    let mut seen: BTreeMap<String, DevToolsSocket> = BTreeMap::new();
    for line in proc_net_unix.lines().skip(1) {
        if let Some(c) = re.captures(line) {
            let name = c[1].to_owned();
            if seen.contains_key(&name) {
                continue;
            }
            let pid = c.get(2).and_then(|m| m.as_str().parse::<u32>().ok());
            let process = pid.map(read_cmdline).unwrap_or_default();
            seen.insert(
                name.clone(),
                DevToolsSocket {
                    socket: name,
                    pid,
                    process,
                },
            );
        }
    }
    seen.into_values().collect()
}

/// Scans `/proc/net/unix` for active WebView/Chrome DevTools abstract sockets.
pub(crate) fn scan_devtools_sockets() -> Vec<DevToolsSocket> {
    std::fs::read_to_string("/proc/net/unix")
        .map(|s| parse_devtools_sockets(&s))
        .unwrap_or_default()
}

/// Selects the best matching DevTools socket by PID or process substring.
pub(crate) fn match_socket(target: Option<&str>, pid: Option<u32>) -> Option<DevToolsSocket> {
    let sockets = scan_devtools_sockets();
    if let Some(p) = pid {
        if let Some(found) = sockets.iter().find(|s| s.pid == Some(p)) {
            return Some(found.clone());
        }
        return Some(DevToolsSocket {
            socket: format!("webview_devtools_remote_{p}"),
            pid: Some(p),
            process: read_cmdline(p),
        });
    }
    if let Some(t) = target
        && let Some(found) = sockets.iter().find(|s| s.process.contains(t))
    {
        return Some(found.clone());
    }
    sockets.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_abstract_devtools_sockets_from_proc_net_unix() {
        let sample = "Num RefCount Protocol Flags Type St Inode Path\n\
            0000: 00000002 00000000 00010000 0001 01 12345 @webview_devtools_remote_4321\n\
            0001: 00000002 00000000 00010000 0001 01 12346 @chrome_devtools_remote\n";
        let found = parse_devtools_sockets(sample);
        assert_eq!(found.len(), 2);
        assert!(
            found
                .iter()
                .any(|s| s.socket == "webview_devtools_remote_4321" && s.pid == Some(4321))
        );
    }
}
