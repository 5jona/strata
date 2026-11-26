//! Mirror preset defaults, env-map mutation helpers, and tools config tests.

use std::collections::BTreeMap;

use strata_protocol::rpc::ToolMirrorPreset;

use crate::tools::config::{
    KEY_APT_MIRROR, KEY_GOPROXY, KEY_NODE_MIRROR, KEY_NPM_REGISTRY, KEY_PIP_INDEX, KEY_PRESET,
    KEY_RUSTUP_DIST, KEY_RUSTUP_ROOT, KEY_UV_INDEX,
};

pub(super) fn apply_preset_defaults(
    env: &mut BTreeMap<String, String>,
    preset: ToolMirrorPreset,
    is_ubuntu: bool,
) {
    match preset {
        ToolMirrorPreset::Tuna => {
            env.insert(KEY_PRESET.to_owned(), "tuna".to_owned());
            env.insert(
                KEY_APT_MIRROR.to_owned(),
                if is_ubuntu {
                    "https://mirrors.tuna.tsinghua.edu.cn/ubuntu-ports".to_owned()
                } else {
                    "https://mirrors.tuna.tsinghua.edu.cn/debian".to_owned()
                },
            );
            env.insert(
                KEY_NODE_MIRROR.to_owned(),
                "https://mirrors.tuna.tsinghua.edu.cn/nodejs-release/".to_owned(),
            );
            env.insert(
                KEY_UV_INDEX.to_owned(),
                "https://pypi.tuna.tsinghua.edu.cn/simple".to_owned(),
            );
            env.insert(
                KEY_PIP_INDEX.to_owned(),
                "https://pypi.tuna.tsinghua.edu.cn/simple".to_owned(),
            );
            env.insert(
                KEY_NPM_REGISTRY.to_owned(),
                "https://registry.npmmirror.com".to_owned(),
            );
            env.insert(
                KEY_RUSTUP_DIST.to_owned(),
                "https://mirrors.tuna.tsinghua.edu.cn/rustup".to_owned(),
            );
            env.insert(
                KEY_RUSTUP_ROOT.to_owned(),
                "https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup".to_owned(),
            );
            env.insert(
                KEY_GOPROXY.to_owned(),
                "https://goproxy.cn,direct".to_owned(),
            );
        }
        ToolMirrorPreset::Official => {
            env.insert(KEY_PRESET.to_owned(), "official".to_owned());
            for k in [
                KEY_APT_MIRROR,
                KEY_NODE_MIRROR,
                KEY_UV_INDEX,
                KEY_PIP_INDEX,
                KEY_NPM_REGISTRY,
                KEY_RUSTUP_DIST,
                KEY_RUSTUP_ROOT,
                KEY_GOPROXY,
            ] {
                env.remove(k);
            }
        }
        ToolMirrorPreset::Custom => {
            env.insert(KEY_PRESET.to_owned(), "custom".to_owned());
        }
    }
}

pub(super) fn parse_preset(raw: Option<&str>) -> ToolMirrorPreset {
    match raw {
        Some("tuna") => ToolMirrorPreset::Tuna,
        Some("custom") => ToolMirrorPreset::Custom,
        _ => ToolMirrorPreset::Official,
    }
}

pub(super) fn non_empty(map: &BTreeMap<String, String>, key: &str) -> Option<String> {
    map.get(key)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

pub(super) fn set_mirror_env(
    map: &mut BTreeMap<String, String>,
    key: &str,
    value: Option<&str>,
    ignore_empty: bool,
) {
    if let Some(raw) = value {
        if ignore_empty && raw.trim().is_empty() {
            return;
        }
        set_opt_env(map, key, Some(raw));
    }
}

pub(super) fn set_opt_env(map: &mut BTreeMap<String, String>, key: &str, value: Option<&str>) {
    if let Some(raw) = value {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            map.remove(key);
        } else {
            map.insert(key.to_owned(), trimmed.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;

    use strata_protocol::rpc::{ToolMirrorPreset, ToolsConfigSetParams};
    use strata_sys::rootfs_io::RootDir;

    use crate::tools::config::{
        DEFAULT_XDG_MISE_CONFIG, apply_config_update, read_config_from_root,
    };

    #[test]
    fn secret_token_is_written_0600_and_never_returned_plaintext() {
        let base = std::env::temp_dir().join(format!("strata-tools-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("etc/strata/mise")).expect("mkdir");
        std::fs::write(base.join("etc/os-release"), "ID=debian\n").expect("os-release");
        let root = RootDir::open(&base).expect("open root");

        apply_config_update(
            "main",
            &root,
            ToolsConfigSetParams {
                mirror_preset: Some(ToolMirrorPreset::Tuna),
                apt_mirror_url: Some(String::new()),
                github_token: Some("ghp_supersecret1234".to_owned()),
                jobs: Some(6),
                no_proxy: Some("localhost,127.0.0.1".to_owned()),
                ..Default::default()
            },
        )
        .expect("apply");

        let meta =
            std::fs::metadata(base.join("etc/strata/mise/secrets.env")).expect("secret meta");
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);

        let res = read_config_from_root("main", &root, None, Vec::new()).expect("read config");
        assert!(res.has_github_token);
        assert_eq!(res.mirror_preset, ToolMirrorPreset::Tuna);
        assert_eq!(
            res.apt_mirror_url.as_deref(),
            Some("https://mirrors.tuna.tsinghua.edu.cn/debian")
        );
        assert_eq!(res.no_proxy.as_deref(), Some("localhost,127.0.0.1"));
        assert_eq!(res.jobs, 6);
        assert_eq!(res.config_path.as_deref(), Some(DEFAULT_XDG_MISE_CONFIG));
        let serialized = serde_json::to_string(&res).expect("serialize");
        assert!(!serialized.contains("ghp_supersecret1234"));

        let mut repls = BTreeMap::new();
        repls.insert("github.com".to_owned(), "gh.mirror.test".to_owned());
        apply_config_update(
            "main",
            &root,
            ToolsConfigSetParams {
                mise_config_toml: Some(res.mise_config_toml),
                jobs: Some(8),
                url_replacements: Some(repls),
                ..Default::default()
            },
        )
        .expect("apply merged toml");
        let res2 = read_config_from_root("main", &root, None, Vec::new()).expect("read config2");
        assert_eq!(res2.jobs, 8);
        assert_eq!(
            res2.url_replacements.get("github.com").map(String::as_str),
            Some("gh.mirror.test")
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ubuntu_preset_detects_usr_lib_os_release_when_etc_is_symlink() {
        let base = std::env::temp_dir().join(format!("strata-tools-ubuntu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("etc/strata/mise")).expect("mkdir etc");
        std::fs::create_dir_all(base.join("usr/lib")).expect("mkdir usr/lib");
        std::fs::write(base.join("usr/lib/os-release"), "ID=ubuntu\n").expect("write os-release");
        std::os::unix::fs::symlink("../usr/lib/os-release", base.join("etc/os-release"))
            .expect("symlink");
        let root = RootDir::open(&base).expect("open root");

        apply_config_update(
            "main",
            &root,
            ToolsConfigSetParams {
                mirror_preset: Some(ToolMirrorPreset::Tuna),
                ..Default::default()
            },
        )
        .expect("apply");

        let res = read_config_from_root("main", &root, None, Vec::new()).expect("read config");
        assert_eq!(
            res.apt_mirror_url.as_deref(),
            Some("https://mirrors.tuna.tsinghua.edu.cn/ubuntu-ports")
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
