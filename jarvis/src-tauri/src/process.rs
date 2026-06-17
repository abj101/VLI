//! Child-process helpers (Windows console flash suppression).

use std::ffi::OsStr;
use std::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Avoid a visible console when spawning shell children from the GUI app.
pub fn hide_console(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// `Command::new` with [`hide_console`] applied.
pub fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    hide_console(&mut cmd);
    cmd
}
