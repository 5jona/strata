use std::cell::RefCell;

use super::*;

#[derive(Debug, Default)]
struct FakeRunner {
    calls: RefCell<Vec<Vec<String>>>,
    responses: RefCell<Vec<CommandOutput>>,
}

impl FakeRunner {
    fn with_responses(responses: Vec<CommandOutput>) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            responses: RefCell::new(responses),
        }
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, _program: &str, args: &[String]) -> Result<CommandOutput, AdbError> {
        self.calls.borrow_mut().push(args.to_vec());
        let mut list = self.responses.borrow_mut();
        if list.is_empty() {
            return Ok(CommandOutput {
                status_code: Some(0),
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        Ok(list.remove(0))
    }
}

fn ok_out(stdout: &str) -> CommandOutput {
    CommandOutput {
        status_code: Some(0),
        success: true,
        stdout: stdout.to_owned(),
        stderr: String::new(),
    }
}

#[test]
fn connects_fetches_token_and_disconnects() {
    let runner = FakeRunner::with_responses(vec![
        ok_out(""),
        ok_out("0123456789abcdef0123456789abcdef\n"),
        ok_out(""),
    ]);
    let mut mgr = AdbManager::with_runner("adb", runner);
    let tunnel = mgr
        .connect_device("emulator-5554", Some(7681), Some(17681))
        .unwrap();
    assert_eq!(tunnel.local_port, 17681);
    assert_eq!(tunnel.base_url, "http://127.0.0.1:17681");
    assert_eq!(
        tunnel.token.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
    assert!(mgr.disconnect_device("emulator-5554").unwrap());
    assert!(!mgr.disconnect_device("emulator-5554").unwrap());
}

#[test]
fn forwards_lists_and_removes_device_ports() {
    let runner = FakeRunner::with_responses(vec![
        ok_out(""),
        ok_out("emulator-5554 tcp:22220 tcp:2222\n192.0.2.20:5555 tcp:17682 tcp:7681\n"),
        ok_out(""),
    ]);
    let mut mgr = AdbManager::with_runner("adb", runner);

    let first = mgr
        .forward_device_port("emulator-5554", 2222, Some(22220))
        .unwrap();
    assert_eq!(first.local_port, 22220);
    assert_eq!(first.remote_port, 2222);
    assert_eq!(first.local_spec, "tcp:22220");
    assert_eq!(first.remote_spec, "tcp:2222");

    // Repeating without explicit local port reuses cached forward without spawning adb.
    let reused = mgr
        .forward_device_port("emulator-5554", 2222, None)
        .unwrap();
    assert_eq!(reused, first);

    let listed = mgr.list_port_forwards(Some("emulator-5554")).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].local_port, 22220);

    assert!(mgr.remove_port_forward("emulator-5554", 22220).unwrap());

    let calls = mgr.runner.calls.borrow();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[0],
        vec!["-s", "emulator-5554", "forward", "tcp:22220", "tcp:2222"]
    );
    assert_eq!(calls[1], vec!["forward", "--list"]);
    assert_eq!(
        calls[2],
        vec!["-s", "emulator-5554", "forward", "--remove", "tcp:22220"]
    );
}

#[test]
fn rejects_zero_port_and_invalid_serial_on_forward() {
    let mut mgr = AdbManager::with_runner("adb", FakeRunner::default());
    assert!(matches!(
        mgr.forward_device_port("emulator-5554", 0, None),
        Err(AdbError::InvalidPort(0))
    ));
    assert!(matches!(
        mgr.forward_device_port("bad serial!", 2222, None),
        Err(AdbError::InvalidSerial(_))
    ));
    assert!(matches!(
        mgr.remove_port_forward("emulator-5554", 0),
        Err(AdbError::InvalidPort(0))
    ));
}
