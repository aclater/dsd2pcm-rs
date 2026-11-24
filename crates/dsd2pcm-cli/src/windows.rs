//! A double-click in Explorer starts the tool with no arguments in a console of its own,
//! which closes as soon as the process ends. In that case the tool says how to run it and
//! waits for Enter before it ends with the usual status for missing arguments.
//!
//! The test: no arguments, the console's process list holds only this process, and the
//! console has a window. A shell, and every process in a pipeline started from one, shares
//! its console, so the list holds more than one process. A process started without a
//! console (`DETACHED_PROCESS`) gets 0 from `GetConsoleProcessList`. A console created
//! without a window (`CREATE_NO_WINDOW`) has no window handle. None of these waits.

/// What `main` does before parsing the arguments.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Run as on every other platform.
    Run,
    /// Print [`MESSAGE`], wait for Enter, then end as for missing arguments.
    Pause,
}

/// Shown in the console of a double-clicked `dsd2pcm.exe`.
pub const MESSAGE: &str = "dsd2pcm is a command-line tool. Open PowerShell or Terminal, then run: \
dsd2pcm IN.dsf OUT.wav (dsd2pcm --help for options).

Press Enter to close this window.";

/// The decision, apart from the system calls: `console_count` is what
/// `GetConsoleProcessList` returned (0 when there is no console) and `console_window`
/// whether `GetConsoleWindow` returned a window.
pub fn action(args_empty: bool, console_count: u32, console_window: bool) -> Action {
    if args_empty && console_count == 1 && console_window {
        Action::Pause
    } else {
        Action::Run
    }
}

#[cfg(windows)]
mod sys {
    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
        pub fn GetConsoleWindow() -> *mut core::ffi::c_void;
    }
}

/// [`action`] for this process.
#[cfg(windows)]
pub fn this_process() -> Action {
    if std::env::args_os().len() > 1 {
        return Action::Run;
    }
    let mut ids = [0u32; 2];
    // SAFETY: `ids` is a writable buffer of `ids.len()` DWORDs that outlives the call, and
    // the function writes at most that many (it only returns the count when the buffer is
    // too small). `GetConsoleWindow` takes no arguments; the handle is only compared with
    // null, never used.
    let (count, window) = unsafe {
        (
            sys::GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32),
            !sys::GetConsoleWindow().is_null(),
        )
    };
    action(true, count, window)
}

/// Prints [`MESSAGE`] and waits until a line or end of file arrives on standard input.
#[cfg(windows)]
pub fn explain_and_wait() {
    eprintln!("{MESSAGE}");
    let _ = std::io::stdin().read_line(&mut String::new());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pauses_only_alone_in_a_windowed_console_without_arguments() {
        assert_eq!(action(true, 1, true), Action::Pause);
        // Started from a shell or in a pipeline: the console is shared.
        assert_eq!(action(true, 2, true), Action::Run);
        assert_eq!(action(true, 5, true), Action::Run);
        // DETACHED_PROCESS: no console.
        assert_eq!(action(true, 0, false), Action::Run);
        // CREATE_NO_WINDOW: a console of its own, but no window.
        assert_eq!(action(true, 1, false), Action::Run);
        // Arguments: always as on other platforms.
        assert_eq!(action(false, 1, true), Action::Run);
        assert_eq!(action(false, 2, true), Action::Run);
        assert_eq!(action(false, 0, false), Action::Run);
    }

    #[test]
    fn message_names_a_command_that_runs() {
        assert!(MESSAGE.contains("dsd2pcm IN.dsf OUT.wav"));
        assert!(MESSAGE.contains("dsd2pcm --help"));
        assert!(MESSAGE.ends_with("Press Enter to close this window."));
    }
}
