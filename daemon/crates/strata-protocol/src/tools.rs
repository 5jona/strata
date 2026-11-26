//! Pure validation, blocklist enforcement, and secret redaction for container tool management.

use thiserror::Error;

use crate::rpc::ToolManagerKind;

const MAX_SPEC_LEN: usize = 160;
const MAX_URL_LEN: usize = 256;
const MAX_SECRET_LEN: usize = 256;

/// Packages forbidden inside ColorOS/GKI Strata containers (ADR-005 / ADR-008).
const FORBIDDEN_INSTALL_PACKAGES: [&str; 6] = [
    "openssh",
    "openssh-server",
    "openssh-sftp-server-privsep",
    "sshd",
    "sudo",
    "policykit-1",
];

/// Core system/transport packages protected against accidental uninstall via UI/RPC.
const PROTECTED_UNINSTALL_PACKAGES: [&str; 6] = [
    "dropbear",
    "dropbear-bin",
    "openssh-sftp-server",
    "ca-certificates",
    "libc6",
    "bash",
];

/// Validation errors for tool package specs, mirror URLs, and credentials.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ToolValidationError {
    #[error("package list must not be empty")]
    EmptyPackageList,
    #[error("invalid package spec '{0}': {1}")]
    InvalidPackageSpec(String, &'static str),
    #[error("package '{0}' is forbidden on ColorOS/GKI Strata containers ({1})")]
    ForbiddenPackage(String, &'static str),
    #[error("invalid URL '{0}': {1}")]
    InvalidUrl(String, &'static str),
    #[error("invalid secret token: {0}")]
    InvalidSecret(&'static str),
    #[error("invalid release age '{0}': expected duration like 24h, 7d, or 0s")]
    InvalidReleaseAge(String),
}

/// Validates a package specification for `manager`, rejecting shell metacharacters,
/// flag injection (`-...`), path traversal (`..`), and ColorOS-unsafe packages.
pub fn validate_package_spec(
    manager: ToolManagerKind,
    raw: &str,
    is_install: bool,
) -> Result<String, ToolValidationError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ToolValidationError::InvalidPackageSpec(
            raw.to_owned(),
            "must not be empty",
        ));
    }
    if trimmed.len() > MAX_SPEC_LEN {
        return Err(ToolValidationError::InvalidPackageSpec(
            trimmed.to_owned(),
            "exceeds 160 characters",
        ));
    }
    if trimmed.starts_with('-') {
        return Err(ToolValidationError::InvalidPackageSpec(
            trimmed.to_owned(),
            "must not start with '-'",
        ));
    }
    if trimmed.contains("..") {
        return Err(ToolValidationError::InvalidPackageSpec(
            trimmed.to_owned(),
            "must not contain '..'",
        ));
    }
    if trimmed.bytes().any(|b| {
        b.is_ascii_whitespace()
            || b.is_ascii_control()
            || matches!(
                b,
                b';' | b'&'
                    | b'|'
                    | b'$'
                    | b'`'
                    | b'('
                    | b')'
                    | b'<'
                    | b'>'
                    | b'\\'
                    | b'\''
                    | b'"'
                    | b'*'
                    | b'?'
                    | b'{'
                    | b'}'
                    | b'~'
                    | b'#'
            )
    }) {
        return Err(ToolValidationError::InvalidPackageSpec(
            trimmed.to_owned(),
            "contains disallowed shell metacharacters or whitespace",
        ));
    }

    let valid_chars = match manager {
        ToolManagerKind::Apt
        | ToolManagerKind::Apk
        | ToolManagerKind::Pacman
        | ToolManagerKind::Dnf => trimmed.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(b, b'+' | b'.' | b'_' | b'-' | b'=' | b':')
        }),
        ToolManagerKind::Mise => trimmed.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'.' | b'_' | b'-' | b'/' | b':' | b'@' | b'+' | b'[' | b']' | b',' | b'='
                )
        }),
        ToolManagerKind::Uv => trimmed.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(b, b'.' | b'_' | b'-' | b'@' | b'[' | b']' | b',' | b'=')
        }),
        ToolManagerKind::Bundle => trimmed
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
        ToolManagerKind::System | ToolManagerKind::Manual => {
            return Err(ToolValidationError::InvalidPackageSpec(
                trimmed.to_owned(),
                "read-only manager does not accept install/uninstall mutations",
            ));
        }
    };
    if !valid_chars {
        return Err(ToolValidationError::InvalidPackageSpec(
            trimmed.to_owned(),
            "contains characters unsupported by target manager",
        ));
    }

    let base = normalize_package_base(trimmed);
    if is_install && FORBIDDEN_INSTALL_PACKAGES.contains(&base.as_str()) {
        return Err(ToolValidationError::ForbiddenPackage(
            trimmed.to_owned(),
            "blocked by ADR-005/ADR-008 ColorOS watchdog and root-default policy",
        ));
    }
    if !is_install && PROTECTED_UNINSTALL_PACKAGES.contains(&base.as_str()) {
        return Err(ToolValidationError::ForbiddenPackage(
            trimmed.to_owned(),
            "essential container runtime/SSH package cannot be removed",
        ));
    }

    Ok(trimmed.to_owned())
}

/// Validates a non-empty list of package specs for install/uninstall mutations.
pub fn validate_package_list(
    manager: ToolManagerKind,
    packages: &[String],
    is_install: bool,
) -> Result<Vec<String>, ToolValidationError> {
    if packages.is_empty() {
        return Err(ToolValidationError::EmptyPackageList);
    }
    packages
        .iter()
        .map(|p| validate_package_spec(manager, p, is_install))
        .collect()
}

/// Validates an HTTP/HTTPS mirror or proxy URL without whitespace or control characters.
pub fn validate_http_url(raw: &str) -> Result<String, ToolValidationError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_URL_LEN {
        return Err(ToolValidationError::InvalidUrl(
            raw.to_owned(),
            "must be 1..=256 characters",
        ));
    }
    if !trimmed.starts_with("https://") && !trimmed.starts_with("http://") {
        return Err(ToolValidationError::InvalidUrl(
            trimmed.to_owned(),
            "must start with https:// or http://",
        ));
    }
    if trimmed.bytes().any(|b| {
        b.is_ascii_whitespace()
            || b.is_ascii_control()
            || matches!(b, b'\'' | b'"' | b'$' | b'`' | b';' | b'\\')
    }) {
        return Err(ToolValidationError::InvalidUrl(
            trimmed.to_owned(),
            "contains disallowed characters",
        ));
    }
    Ok(trimmed.to_owned())
}

/// Validates a proxy / no_proxy list string.
pub fn validate_proxy_list(raw: &str) -> Result<String, ToolValidationError> {
    let trimmed = raw.trim();
    if trimmed.len() > MAX_URL_LEN {
        return Err(ToolValidationError::InvalidUrl(
            trimmed.to_owned(),
            "exceeds 256 characters",
        ));
    }
    if !trimmed.bytes().all(|b| {
        b.is_ascii_alphanumeric() || matches!(b, b'.' | b',' | b':' | b'-' | b'_' | b'/' | b'*')
    }) {
        return Err(ToolValidationError::InvalidUrl(
            trimmed.to_owned(),
            "invalid characters in no_proxy list",
        ));
    }
    Ok(trimmed.to_owned())
}

/// Validates a GitHub/registry token secret string.
pub fn validate_token_secret(raw: &str) -> Result<String, ToolValidationError> {
    let trimmed = raw.trim();
    if trimmed.len() > MAX_SECRET_LEN {
        return Err(ToolValidationError::InvalidSecret("exceeds 256 characters"));
    }
    if !trimmed
        .bytes()
        .all(|b| b.is_ascii_graphic() && !matches!(b, b'\'' | b'"' | b'\\' | b'$' | b'`'))
    {
        return Err(ToolValidationError::InvalidSecret(
            "must contain only printable ASCII characters without quotes or escapes",
        ));
    }
    Ok(trimmed.to_owned())
}

/// Validates a `minimum_release_age` duration string (`0s`, `24h`, `7d`, `1mo`, `1y`).
pub fn validate_release_age(raw: &str) -> Result<String, ToolValidationError> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > 16
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(ToolValidationError::InvalidReleaseAge(raw.to_owned()));
    }
    Ok(trimmed.to_owned())
}

/// Masks a secret token for safe API/UI display (never exposing full credentials).
#[must_use]
pub fn redact_secret(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 8 {
        return "****".to_owned();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}...{tail}")
}

fn normalize_package_base(spec: &str) -> String {
    let without_eq = spec.split('=').next().unwrap_or(spec);
    let without_opts = without_eq.split('[').next().unwrap_or(without_eq);
    let without_at = match without_opts.rfind('@') {
        Some(pos)
            if pos > without_opts.rfind('/').unwrap_or(0)
                && !without_opts[..pos].ends_with(':') =>
        {
            &without_opts[..pos]
        }
        _ => without_opts,
    };
    let without_backend = match without_at.split_once(':') {
        Some((prefix, rest))
            if matches!(
                prefix,
                "aqua" | "cargo" | "core" | "gem" | "go" | "http" | "npm" | "pipx" | "spm" | "ubi"
            ) || rest.contains('/') =>
        {
            rest
        }
        // APT multiarch qualifier like `openssh-server:arm64`.
        Some((pkg, _arch)) => pkg,
        None => without_at,
    };
    let leaf = without_backend
        .rsplit('/')
        .next()
        .unwrap_or(without_backend);
    leaf.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_package_specs_and_blocks_shell_injection() {
        assert_eq!(
            validate_package_spec(ToolManagerKind::Mise, "ripgrep", true).expect("ripgrep"),
            "ripgrep"
        );
        assert_eq!(
            validate_package_spec(ToolManagerKind::Mise, "aqua:BurntSushi/ripgrep@14", true)
                .expect("aqua"),
            "aqua:BurntSushi/ripgrep@14"
        );
        assert_eq!(
            validate_package_spec(ToolManagerKind::Mise, "npm:@biomejs/biome@2", true)
                .expect("scoped npm"),
            "npm:@biomejs/biome@2"
        );
        assert_eq!(
            validate_package_spec(ToolManagerKind::Apt, "python3-dev", true).expect("apt"),
            "python3-dev"
        );
        assert_eq!(
            validate_package_spec(ToolManagerKind::Uv, "ruff==0.9.1", true).expect("uv"),
            "ruff==0.9.1"
        );

        for bad in [
            "",
            "--help",
            "-rf",
            "../evil",
            "jq; rm -rf /",
            "jq && id",
            "jq | cat",
            "jq$(whoami)",
            "jq\nid",
            "jq `id`",
        ] {
            assert!(
                validate_package_spec(ToolManagerKind::Mise, bad, true).is_err(),
                "expected rejection for '{bad}'"
            );
            assert!(
                validate_package_spec(ToolManagerKind::Apt, bad, true).is_err(),
                "expected apt rejection for '{bad}'"
            );
        }
    }

    #[test]
    fn enforces_coloros_openssh_blocklist_and_protected_uninstall() {
        for forbidden in [
            "openssh",
            "openssh-server",
            "openssh-server=1:9.6",
            "sudo",
            "sshd",
        ] {
            let err = validate_package_spec(ToolManagerKind::Apt, forbidden, true)
                .expect_err("must block forbidden package");
            assert!(matches!(err, ToolValidationError::ForbiddenPackage(_, _)));
        }
        assert!(matches!(
            validate_package_spec(ToolManagerKind::Apk, "openssh", true),
            Err(ToolValidationError::ForbiddenPackage(_, _))
        ));
        assert!(matches!(
            validate_package_spec(ToolManagerKind::Pacman, "openssh", true),
            Err(ToolValidationError::ForbiddenPackage(_, _))
        ));
        assert!(validate_package_spec(ToolManagerKind::Apt, "openssh-client", true).is_ok());

        for protected in ["dropbear-bin", "openssh-sftp-server", "ca-certificates"] {
            let err = validate_package_spec(ToolManagerKind::Apt, protected, false)
                .expect_err("must protect base package from uninstall");
            assert!(matches!(err, ToolValidationError::ForbiddenPackage(_, _)));
        }
    }

    #[test]
    fn redacts_secrets_and_validates_urls() {
        assert_eq!(redact_secret(""), "");
        assert_eq!(redact_secret("short"), "****");
        assert_eq!(redact_secret("ghp_1234567890abcd"), "ghp_...abcd");
        assert!(validate_http_url("https://mirrors.tuna.tsinghua.edu.cn/ubuntu-ports").is_ok());
        assert!(validate_http_url("ftp://example.com").is_err());
        assert!(validate_http_url("https://example.com/$(id)").is_err());
        assert!(validate_token_secret("ghp_validToken_123").is_ok());
        assert!(validate_token_secret("bad token\n").is_err());
    }
}
