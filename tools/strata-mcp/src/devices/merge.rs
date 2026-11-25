//! Correlates ADB devices, `~/.ssh/config` hosts, and `devices.json` entries into `DeviceRecord`s.

use std::collections::HashMap;

use super::discovery::{AdbEntry, ConfigDevice, DeviceRecord, SshEntry, is_ip_literal, tcpip_ip};

/// Correlates devices from `devices.json`, `adb devices -l`, and `~/.ssh/config`.
pub(crate) fn merge_devices(
    cfg: &HashMap<String, ConfigDevice>,
    adb_list: &[AdbEntry],
    ssh_list: &[SshEntry],
) -> Vec<DeviceRecord> {
    let mut records: Vec<DeviceRecord> = Vec::new();
    let mut by_serial: HashMap<String, usize> = HashMap::new();
    let mut by_ssh: HashMap<String, usize> = HashMap::new();

    for (alias, item) in cfg {
        let prefer = item
            .prefer_transport
            .as_deref()
            .map(str::to_ascii_lowercase)
            .filter(|s| s == "adb" || s == "ssh")
            .unwrap_or_else(|| "ssh".to_owned());
        let idx = records.len();
        records.push(DeviceRecord {
            id: alias.clone(),
            alias: Some(alias.clone()),
            model: item.model.clone(),
            product: None,
            adb_serial: item.adb_serial.clone(),
            adb_state: None,
            adb_transports: Vec::new(),
            adb_usb_serial: None,
            adb_tcpip_serial: None,
            ssh_host: item.ssh_host.clone(),
            ssh_hostname: None,
            ssh_port: item.ssh_port,
            ssh_user: item.ssh_user.clone(),
            prefer_transport: prefer,
            sources: vec!["devices_json".to_owned()],
        });
        if let Some(s) = item.adb_serial.as_ref() {
            by_serial.insert(s.clone(), idx);
        }
        if let Some(h) = item.ssh_host.as_ref() {
            by_ssh.insert(h.clone(), idx);
        }
    }

    for adb in adb_list {
        let idx = if let Some(&i) = by_serial.get(&adb.serial) {
            i
        } else {
            let i = records.len();
            records.push(DeviceRecord {
                id: adb.serial.clone(),
                alias: None,
                model: adb.model.clone(),
                product: adb.product.clone(),
                adb_serial: Some(adb.serial.clone()),
                adb_state: None,
                adb_transports: Vec::new(),
                adb_usb_serial: None,
                adb_tcpip_serial: None,
                ssh_host: None,
                ssh_hostname: None,
                ssh_port: None,
                ssh_user: None,
                prefer_transport: "ssh".to_owned(),
                sources: Vec::new(),
            });
            by_serial.insert(adb.serial.clone(), i);
            i
        };
        let rec = &mut records[idx];
        if rec.model.is_none() {
            rec.model.clone_from(&adb.model);
        }
        if rec.product.is_none() {
            rec.product.clone_from(&adb.product);
        }
        push_unique(&mut rec.sources, "adb");
        push_unique(&mut rec.adb_transports, &adb.transport);
        if adb.transport == "usb" {
            rec.adb_usb_serial = Some(adb.serial.clone());
        } else {
            rec.adb_tcpip_serial = Some(adb.serial.clone());
        }
        if rec.adb_state.as_deref() != Some("device") || adb.state == "device" {
            rec.adb_state = Some(adb.state.clone());
        }
    }

    for ssh in ssh_list {
        let matched = by_ssh.get(&ssh.host).copied().or_else(|| {
            let ip = ssh.hostname.as_deref().filter(|h| is_ip_literal(h))?;
            records.iter().position(|r| {
                r.adb_tcpip_serial.as_deref().and_then(tcpip_ip).as_deref() == Some(ip)
            })
        });
        if let Some(idx) = matched {
            let rec = &mut records[idx];
            rec.ssh_host.get_or_insert_with(|| ssh.host.clone());
            rec.ssh_hostname
                .get_or_insert_with(|| ssh.hostname.clone().unwrap_or_default());
            if rec.ssh_port.is_none() {
                rec.ssh_port = ssh.port;
            }
            if rec.ssh_user.is_none() {
                rec.ssh_user.clone_from(&ssh.user);
            }
            push_unique(&mut rec.sources, "ssh_config");
        } else {
            let idx = records.len();
            records.push(DeviceRecord {
                id: ssh.host.clone(),
                alias: Some(ssh.host.clone()),
                model: None,
                product: None,
                adb_serial: None,
                adb_state: None,
                adb_transports: Vec::new(),
                adb_usb_serial: None,
                adb_tcpip_serial: None,
                ssh_host: Some(ssh.host.clone()),
                ssh_hostname: ssh.hostname.clone(),
                ssh_port: ssh.port,
                ssh_user: ssh.user.clone(),
                prefer_transport: "ssh".to_owned(),
                sources: vec!["ssh_config".to_owned()],
            });
            by_ssh.insert(ssh.host.clone(), idx);
        }
    }
    records
}

fn push_unique(list: &mut Vec<String>, val: &str) {
    if !list.iter().any(|x| x == val) {
        list.push(val.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::devices::discovery::{parse_adb_devices_output, parse_ssh_config_text};

    #[test]
    fn correlates_adb_tcpip_and_ssh_config_by_ip() {
        let adb_txt = "List of devices attached\n\
            192.0.2.10:5555 device product:demo model:DemoTab transport_id:2\n\
            EMU0001 device product:emu model:Emulator transport_id:1\n";
        let adb = parse_adb_devices_output(adb_txt);
        let ssh_txt = "Host demo-tab\n  HostName 192.0.2.10\n  Port 2222\n  User root\n";
        let ssh = parse_ssh_config_text(ssh_txt, &HashSet::new());
        let merged = merge_devices(&HashMap::new(), &adb, &ssh);
        assert_eq!(merged.len(), 2);
        let tab = merged
            .iter()
            .find(|r| r.ssh_host.as_deref() == Some("demo-tab"))
            .expect("correlated");
        assert_eq!(tab.adb_tcpip_serial.as_deref(), Some("192.0.2.10:5555"));
    }
}
