//! SELinux extended attribute labeling for container backing images.

use std::ffi::{CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;

use crate::error::{SysError, SysResult};

const SELINUX_XATTR_NAME: &str = "security.selinux";
const CHCON_BIN: &str = "/system/bin/chcon";

/// Applies the requested SELinux context to `path` via `lsetxattr`, falling back to
/// `/system/bin/chcon` if direct xattr writes are blocked by policy.
pub fn set_file_context(path: &Path, context: &str) -> SysResult<()> {
    let c_path = os_to_cstring(path.as_os_str())?;
    let c_attr = CString::new(SELINUX_XATTR_NAME)
        .map_err(|_| SysError::InvalidCString(SELINUX_XATTR_NAME.to_owned()))?;
    let mut value = context.as_bytes().to_vec();
    value.push(0);

    // SAFETY: `c_path`, `c_attr`, and `value` are valid NUL-terminated buffers with exact length.
    let rc = unsafe {
        libc::lsetxattr(
            c_path.as_ptr(),
            c_attr.as_ptr(),
            value.as_ptr().cast::<libc::c_void>(),
            value.len(),
            0,
        )
    };
    if rc == 0 {
        return Ok(());
    }

    if Path::new(CHCON_BIN).exists() {
        let status = Command::new(CHCON_BIN)
            .arg(context)
            .arg(path)
            .status()
            .map_err(|e| SysError::path_io("chcon", path, e))?;
        if status.success() {
            return Ok(());
        }
    }

    Err(SysError::path_os("lsetxattr(security.selinux)", path))
}

fn os_to_cstring(s: &OsStr) -> SysResult<CString> {
    CString::new(s.as_bytes())
        .map_err(|_| SysError::InvalidCString(s.to_string_lossy().into_owned()))
}
