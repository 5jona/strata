//! Linux `/dev/uinput` Multitouch Protocol B virtual touchscreen ioctl driver.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{DisplayGeometry, logical_to_physical};

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0x00;
const BTN_TOUCH: u16 = 0x14A;
const INPUT_PROP_DIRECT: libc::c_ulong = 0x01;
const BUS_VIRTUAL: u16 = 0x06;

const ABS_MT_SLOT: u16 = 0x2F;
const ABS_MT_TOUCH_MAJOR: u16 = 0x30;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_PRESSURE: u16 = 0x3A;

const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
const UI_DEV_SETUP: libc::c_ulong = 0x405C_5503;
const UI_ABS_SETUP: libc::c_ulong = 0x401C_5504;
const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
const UI_SET_ABSBIT: libc::c_ulong = 0x4004_5567;
const UI_SET_PROPBIT: libc::c_ulong = 0x4004_556E;

#[repr(C)]
struct InputEvent {
    tv_sec: libc::c_long,
    tv_usec: libc::c_long,
    ev_type: u16,
    code: u16,
    value: i32,
}

#[repr(C)]
struct UinputSetup {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
    name: [u8; 80],
    ff_effects_max: u32,
}

#[repr(C)]
struct UinputAbsSetup {
    code: u16,
    _pad: u16,
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

/// Active `/dev/uinput` multitouch device handle.
pub(crate) struct UInputTouch {
    file: std::fs::File,
    geom: DisplayGeometry,
    tracking_id: i32,
}

impl UInputTouch {
    /// Opens `/dev/uinput` and registers a virtual direct multitouch panel.
    pub(crate) fn open(geom: DisplayGeometry) -> Result<Self, String> {
        let file = OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .map_err(|e| {
                format!("open /dev/uinput failed (enable extraCharDevs=['uinput']): {e}")
            })?;
        let fd = file.as_raw_fd();

        // SAFETY: Valid ioctl requests and value arguments for Linux uinput fd.
        unsafe {
            for (req, val) in [
                (UI_SET_EVBIT, libc::c_ulong::from(EV_KEY)),
                (UI_SET_EVBIT, libc::c_ulong::from(EV_ABS)),
                (UI_SET_EVBIT, libc::c_ulong::from(EV_SYN)),
                (UI_SET_KEYBIT, libc::c_ulong::from(BTN_TOUCH)),
                (UI_SET_PROPBIT, INPUT_PROP_DIRECT),
            ] {
                if libc::ioctl(fd, req as _, val) < 0 {
                    return Err(format!(
                        "uinput ioctl {req:#x} failed: {}",
                        std::io::Error::last_os_error()
                    ));
                }
            }
            for (code, lo, hi) in [
                (ABS_MT_SLOT, 0, 9),
                (ABS_MT_TRACKING_ID, 0, 65535),
                (ABS_MT_POSITION_X, 0, geom.phys_w.max(2) - 1),
                (ABS_MT_POSITION_Y, 0, geom.phys_h.max(2) - 1),
                (ABS_MT_TOUCH_MAJOR, 0, 255),
                (ABS_MT_PRESSURE, 0, 255),
            ] {
                if libc::ioctl(fd, UI_SET_ABSBIT as _, libc::c_ulong::from(code)) < 0 {
                    return Err(format!(
                        "UI_SET_ABSBIT {code:#x} failed: {}",
                        std::io::Error::last_os_error()
                    ));
                }
                let abs = UinputAbsSetup {
                    code,
                    _pad: 0,
                    value: 0,
                    minimum: lo,
                    maximum: hi,
                    fuzz: 0,
                    flat: 0,
                    resolution: 0,
                };
                if libc::ioctl(fd, UI_ABS_SETUP as _, &abs as *const UinputAbsSetup) < 0 {
                    return Err(format!(
                        "UI_ABS_SETUP {code:#x} failed: {}",
                        std::io::Error::last_os_error()
                    ));
                }
            }
            let mut name = [0u8; 80];
            let label = b"strata-virtual-touch";
            name[..label.len()].copy_from_slice(label);
            let setup = UinputSetup {
                bustype: BUS_VIRTUAL,
                vendor: 0x18D1,
                product: 0x4EE7,
                version: 1,
                name,
                ff_effects_max: 0,
            };
            libc::ioctl(fd, UI_DEV_SETUP as _, &setup as *const UinputSetup);
            if libc::ioctl(fd, UI_DEV_CREATE as _) < 0 {
                return Err(format!(
                    "UI_DEV_CREATE failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        std::thread::sleep(Duration::from_millis(80));
        Ok(Self {
            file,
            geom,
            tracking_id: 1,
        })
    }

    fn emit(&mut self, events: &[(u16, u16, i32)]) -> Result<(), String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let sec = now.as_secs() as libc::c_long;
        let usec = now.subsec_micros() as libc::c_long;
        let mut raw = Vec::with_capacity(events.len() * std::mem::size_of::<InputEvent>());
        for &(ev_type, code, value) in events {
            let ev = InputEvent {
                tv_sec: sec,
                tv_usec: usec,
                ev_type,
                code,
                value,
            };
            // SAFETY: InputEvent is #[repr(C)] plain data with no uninitialized padding bytes.
            let slice = unsafe {
                std::slice::from_raw_parts(
                    (&raw const ev).cast::<u8>(),
                    std::mem::size_of::<InputEvent>(),
                )
            };
            raw.extend_from_slice(slice);
        }
        self.file
            .write_all(&raw)
            .map_err(|e| format!("uinput write failed: {e}"))
    }

    fn down(&mut self, x: i32, y: i32) -> Result<(), String> {
        let (px, py) = logical_to_physical(x, y, self.geom);
        let tid = self.tracking_id;
        self.tracking_id += 1;
        self.emit(&[
            (EV_ABS, ABS_MT_SLOT, 0),
            (EV_ABS, ABS_MT_TRACKING_ID, tid),
            (EV_ABS, ABS_MT_POSITION_X, px),
            (EV_ABS, ABS_MT_POSITION_Y, py),
            (EV_ABS, ABS_MT_TOUCH_MAJOR, 24),
            (EV_ABS, ABS_MT_PRESSURE, 80),
            (EV_KEY, BTN_TOUCH, 1),
            (EV_SYN, SYN_REPORT, 0),
        ])
    }

    fn move_to(&mut self, x: i32, y: i32) -> Result<(), String> {
        let (px, py) = logical_to_physical(x, y, self.geom);
        self.emit(&[
            (EV_ABS, ABS_MT_SLOT, 0),
            (EV_ABS, ABS_MT_POSITION_X, px),
            (EV_ABS, ABS_MT_POSITION_Y, py),
            (EV_SYN, SYN_REPORT, 0),
        ])
    }

    fn up(&mut self) -> Result<(), String> {
        self.emit(&[
            (EV_ABS, ABS_MT_SLOT, 0),
            (EV_ABS, ABS_MT_TRACKING_ID, -1),
            (EV_KEY, BTN_TOUCH, 0),
            (EV_SYN, SYN_REPORT, 0),
        ])
    }

    /// Executes a tap or long press at `(x, y)`.
    pub(crate) fn tap(&mut self, x: i32, y: i32, hold_ms: u64) -> Result<(), String> {
        self.down(x, y)?;
        std::thread::sleep(Duration::from_millis(hold_ms));
        self.up()
    }

    /// Executes a linear swipe with monotonic frame pacing.
    pub(crate) fn swipe(
        &mut self,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        duration_ms: u64,
    ) -> Result<(), String> {
        let steps = (duration_ms / 12).clamp(8, 60) as usize;
        let step_dur = Duration::from_millis((duration_ms / (steps as u64)).max(4));
        let start = Instant::now();
        self.down(x0, y0)?;
        for i in 1..=steps {
            let target = start + step_dur * (i as u32);
            if let Some(rem) = target.checked_duration_since(Instant::now()) {
                std::thread::sleep(rem);
            }
            let t = (i as f64) / (steps as f64);
            let x = (f64::from(x0) + f64::from(x1 - x0) * t).round() as i32;
            let y = (f64::from(y0) + f64::from(y1 - y0) * t).round() as i32;
            self.move_to(x, y)?;
        }
        self.up()
    }

    /// Replays precomputed `(x, y, dt_ms)` touch trajectory points with monotonic pacing.
    pub(crate) fn replay_points(&mut self, pts: &[(i32, i32, u64)]) -> Result<(), String> {
        let Some(&(x0, y0, dt0)) = pts.first() else {
            return Ok(());
        };
        let mut cursor = Instant::now() + Duration::from_millis(dt0);
        self.down(x0, y0)?;
        for &(x, y, dt) in &pts[1..] {
            if let Some(rem) = cursor.checked_duration_since(Instant::now()) {
                std::thread::sleep(rem);
            }
            self.move_to(x, y)?;
            cursor += Duration::from_millis(dt);
        }
        self.up()
    }
}

impl Drop for UInputTouch {
    fn drop(&mut self) {
        // SAFETY: Destroying a previously created uinput virtual device on close.
        unsafe {
            let _ = libc::ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY as _);
        }
    }
}
