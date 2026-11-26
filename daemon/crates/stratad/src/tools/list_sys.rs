//! System package manager inventory helpers (`apt`, `apk`, `pacman`, `dnf`) for `tools.list`.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use strata_protocol::ContainerConfig;
use strata_protocol::rpc::{InstalledToolItem, ToolManagerKind};

use crate::tools::ToolsManager;
use crate::tools::catalog_data::KNOWN_APT_PACKAGES;
use crate::tools::exec::run_container_command_bounded;
use crate::tools::status_parsers_sys::{parse_dpkg_query, parse_generic_sys_pkg_lines};

pub(super) async fn collect_system_packages(
    tools: &ToolsManager,
    container: &str,
    ns_pid: u32,
    cfg: &ContainerConfig,
    env: &BTreeMap<String, String>,
    sys_mgr: Option<ToolManagerKind>,
    items: &mut Vec<InstalledToolItem>,
) {
    match sys_mgr {
        Some(ToolManagerKind::Apt) => {
            let mut cmd = vec![
                "dpkg-query".to_owned(),
                "-W".to_owned(),
                "-f=${Package}\\t${Status}\\t${Version}\\n".to_owned(),
            ];
            for (pkg, _) in KNOWN_APT_PACKAGES {
                cmd.push((*pkg).to_owned());
            }
            if let Ok(out) = run_container_command_bounded(
                &tools.self_exe,
                container,
                ns_pid,
                cfg,
                cmd,
                env.clone(),
                Duration::from_secs(6),
            )
            .await
            {
                items.extend(parse_dpkg_query(&out.stdout, false));
            }
            if let Ok(man_out) = run_container_command_bounded(
                &tools.self_exe,
                container,
                ns_pid,
                cfg,
                vec![
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    "apt-mark showmanual 2>/dev/null | xargs -r dpkg-query -W -f='${Package}\\t${Status}\\t${Version}\\n' 2>/dev/null || true".to_owned(),
                ],
                env.clone(),
                Duration::from_secs(6),
            )
            .await
            {
                let seen: BTreeSet<String> = items.iter().map(|i| i.id.clone()).collect();
                for item in parse_dpkg_query(&man_out.stdout, true) {
                    if !seen.contains(&item.id) {
                        items.push(item);
                    }
                }
            }
        }
        Some(ToolManagerKind::Apk) => {
            if let Ok(out) = run_container_command_bounded(
                &tools.self_exe,
                container,
                ns_pid,
                cfg,
                vec![
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    "apk info -v 2>/dev/null | sed -E 's/-([0-9][^ ]*)$/\\t\\1/' || true"
                        .to_owned(),
                ],
                env.clone(),
                Duration::from_secs(6),
            )
            .await
            {
                items.extend(parse_generic_sys_pkg_lines(
                    &out.stdout,
                    ToolManagerKind::Apk,
                    "apk",
                ));
            }
        }
        Some(ToolManagerKind::Pacman) => {
            if let Ok(out) = run_container_command_bounded(
                &tools.self_exe,
                container,
                ns_pid,
                cfg,
                vec!["pacman".to_owned(), "-Q".to_owned()],
                env.clone(),
                Duration::from_secs(6),
            )
            .await
            {
                items.extend(parse_generic_sys_pkg_lines(
                    &out.stdout,
                    ToolManagerKind::Pacman,
                    "pacman",
                ));
            }
        }
        Some(ToolManagerKind::Dnf) => {
            if let Ok(out) = run_container_command_bounded(
                &tools.self_exe,
                container,
                ns_pid,
                cfg,
                vec![
                    "rpm".to_owned(),
                    "-qa".to_owned(),
                    "--qf".to_owned(),
                    "%{NAME}\\t%{VERSION}\\n".to_owned(),
                ],
                env.clone(),
                Duration::from_secs(6),
            )
            .await
            {
                items.extend(parse_generic_sys_pkg_lines(
                    &out.stdout,
                    ToolManagerKind::Dnf,
                    "dnf",
                ));
            }
        }
        _ => {}
    }
}
