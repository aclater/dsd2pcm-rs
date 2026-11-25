//! Windows: started with no arguments in a console of its own (as by a double-click in
//! Explorer), the tool explains itself and waits for Enter, then exits with 2. Started
//! without a console, or in a console without a window, it does not wait.
#![cfg(windows)]

use std::io::Write;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const MESSAGE: &str = "dsd2pcm is a command-line tool.";

fn spawn(flags: u32, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_dsd2pcm"))
        .args(args)
        .creation_flags(flags)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// Waits up to `secs` for the child to end; kills it and returns `None` if it does not.
fn wait_for(child: &mut Child, secs: u64) -> Option<ExitStatus> {
    let st = wait_for_quietly(child, secs);
    if st.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    st
}

fn stderr_of(child: Child) -> String {
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn own_console_without_arguments_waits_for_enter() {
    let mut child = spawn(CREATE_NEW_CONSOLE, &[]);
    // Standard input stays open and empty: the tool must still be waiting.
    assert_eq!(wait_for_quietly(&mut child, 2), None, "did not wait");
    child.stdin.as_mut().unwrap().write_all(b"\r\n").unwrap();
    let st = wait_for(&mut child, 30).expect("still running after Enter");
    assert_eq!(st.code(), Some(2));
    let err = stderr_of(child);
    assert!(err.contains(MESSAGE), "{err}");
}

#[test]
fn own_console_without_arguments_ends_on_end_of_input() {
    let mut child = spawn(CREATE_NEW_CONSOLE, &[]);
    drop(child.stdin.take());
    let st = wait_for(&mut child, 30).expect("hung on closed standard input");
    assert_eq!(st.code(), Some(2));
    assert!(stderr_of(child).contains(MESSAGE));
}

#[test]
fn own_console_with_arguments_runs_as_usual() {
    let mut child = spawn(CREATE_NEW_CONSOLE, &["--help"]);
    let st = wait_for(&mut child, 30).expect("hung with --help");
    assert_eq!(st.code(), Some(0));
    assert!(!stderr_of(child).contains(MESSAGE));
}

/// No console, or a console without a window: standard input stays open and empty, and the
/// tool must still end at once with the missing-arguments status and no message. (Flags 0
/// are left out: the child then shares this process's console if it has one, and gets a new
/// windowed console, as from Explorer, if it has none.)
#[test]
fn no_console_or_no_window_never_waits() {
    for flags in [DETACHED_PROCESS, CREATE_NO_WINDOW] {
        let mut child = spawn(flags, &[]);
        let st = wait_for(&mut child, 30)
            .unwrap_or_else(|| panic!("waited with creation flags {flags:#x}"));
        assert_eq!(st.code(), Some(2), "flags {flags:#x}");
        let err = stderr_of(child);
        assert!(!err.contains(MESSAGE), "flags {flags:#x}: {err}");
        assert!(err.contains("need IN and OUT"), "flags {flags:#x}: {err}");
    }
}

/// Like [`wait_for`], but leaves the child running when it does not end in time.
fn wait_for_quietly(child: &mut Child, secs: u64) -> Option<ExitStatus> {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if let Some(st) = child.try_wait().unwrap() {
            return Some(st);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}
