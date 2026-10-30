use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

pub struct Sidecar {
    child: Child,
    pub port: u16,
    #[cfg(windows)]
    _job: std::os::windows::io::OwnedHandle,
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn spawn(
    app: &AppHandle,
    exe: &str,
    args: &[String],
    health_path: &str,
    timeout: Duration,
) -> Result<Sidecar, String> {
    let resource_root = app
        .path()
        .resource_dir()
        .map_err(|e| format!("resolve app resources: {e}"))?;
    spawn_at(&resource_root, exe, args, health_path, timeout)
}

pub fn spawn_at(
    resource_root: &Path,
    exe: &str,
    args: &[String],
    health_path: &str,
    timeout: Duration,
) -> Result<Sidecar, String> {
    let port = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("reserve sidecar port: {e}"))?
        .local_addr()
        .map_err(|e| format!("read sidecar port: {e}"))?
        .port();
    let path = resource_root.join("bin").join(exe);
    let program = if path.is_file() {
        path
    } else if cfg!(target_os = "macos") {
        PathBuf::from(
            std::path::Path::new(exe)
                .file_name()
                .unwrap_or(exe.as_ref()),
        )
    } else {
        return Err(format!("sidecar executable not found: {}", path.display()));
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .args(["--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|e| format!("start sidecar: {e}"))?;
    #[cfg(windows)]
    let job = match assign_kill_job(&child) {
        Ok(job) => job,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };

    let mut sidecar = Sidecar {
        child,
        port,
        #[cfg(windows)]
        _job: job,
    };

    let stderr_tail = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    if let Some(out) = sidecar.child.stdout.take() {
        thread::spawn(move || log_output(BufReader::new(out), "stdout", None));
    }
    if let Some(err) = sidecar.child.stderr.take() {
        let tail = Arc::clone(&stderr_tail);
        thread::spawn(move || log_output(BufReader::new(err), "stderr", Some(tail)));
    }
    let url = format!("http://127.0.0.1:{port}{health_path}");
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = sidecar
            .child
            .try_wait()
            .map_err(|e| format!("check sidecar: {e}"))?
        {
            return Err(format!(
                "sidecar exited ({status}); stderr: {}",
                stderr_text(&stderr_tail)
            ));
        }
        if ureq::get(&url)
            .timeout(Duration::from_millis(250))
            .call()
            .is_ok_and(|response| response.status() == 200)
        {
            return Ok(sidecar);
        }
        if Instant::now() >= deadline {
            let _ = sidecar.child.kill();
            let _ = sidecar.child.wait();
            return Err(format!(
                "sidecar health timeout; stderr: {}",
                stderr_text(&stderr_tail)
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn log_output<R: BufRead>(
    reader: R,
    stream: &str,
    tail: Option<Arc<Mutex<std::collections::VecDeque<String>>>>,
) {
    for line in reader.lines().map_while(Result::ok) {
        log::debug!("sidecar {stream}: {line}");
        if let Some(tail) = &tail {
            if let Ok(mut lines) = tail.lock() {
                if lines.len() == 8 {
                    lines.pop_front();
                }
                lines.push_back(line);
            }
        }
    }
}

fn stderr_text(tail: &Mutex<std::collections::VecDeque<String>>) -> String {
    tail.lock()
        .map(|lines| lines.iter().cloned().collect::<Vec<_>>().join(" | "))
        .unwrap_or_else(|_| "stderr unavailable".into())
}

#[cfg(windows)]
fn assign_kill_job(child: &Child) -> Result<std::os::windows::io::OwnedHandle, String> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    unsafe {
        let job = CreateJobObjectW(None, windows::core::PCWSTR::null())
            .map_err(|e| format!("create sidecar job: {e}"))?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if let Err(e) = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as _,
            std::mem::size_of_val(&info) as u32,
        ) {
            let _ = windows::Win32::Foundation::CloseHandle(job);
            return Err(format!("configure sidecar job: {e}"));
        }
        let process = HANDLE(child.as_raw_handle() as _);
        if let Err(e) = AssignProcessToJobObject(job, process) {
            let _ = windows::Win32::Foundation::CloseHandle(job);
            return Err(format!("assign sidecar job: {e}"));
        }
        Ok(OwnedHandle::from_raw_handle(job.0 as _))
    }
}
