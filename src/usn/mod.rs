//! NoirView's side of the optional USN broker service: talk to it over its
//! named pipe, see whether it is installed, and install or remove it (one
//! administrator prompt each). The service itself is `noirview-usn.exe`
//! (`src/bin/noirview-usn.rs`); it only reads the NTFS change journal.
pub mod protocol;

use anyhow::{bail, Result};
use protocol::{Request, Response};
use std::path::Path;

/// What Settings shows about the service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    NotInstalled,
    /// Installed but not running (starting, stopped or failing).
    Stopped,
    Running,
    /// Not available on this system.
    Unsupported,
}

/// One request and its response (blocking; call from a worker thread).
pub fn call(request: &Request) -> Result<Response> {
    #[cfg(windows)]
    {
        use std::io::{BufRead, BufReader, Write};
        let mut attempts = 0;
        let pipe = loop {
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(protocol::PIPE)
            {
                Ok(pipe) => break pipe,
                // 231 = ERROR_PIPE_BUSY: every instance is serving someone.
                Err(e) if e.raw_os_error() == Some(231) && attempts < 50 => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(e) => bail!("The NoirView USN service is not reachable: {e}"),
            }
        };
        let mut line = serde_json::to_string(request)?;
        line.push('\n');
        (&pipe).write_all(line.as_bytes())?;
        let mut answer = String::new();
        BufReader::new(&pipe).read_line(&mut answer)?;
        Ok(serde_json::from_str(&answer)?)
    }
    #[cfg(not(windows))]
    {
        let _ = request;
        bail!("The USN service is available on Windows only")
    }
}

pub fn state() -> State {
    #[cfg(windows)]
    {
        use windows_service::{
            service::{ServiceAccess, ServiceState},
            service_manager::{ServiceManager, ServiceManagerAccess},
        };
        let Ok(manager) =
            ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        else {
            return State::Unsupported;
        };
        match manager.open_service(protocol::SERVICE, ServiceAccess::QUERY_STATUS) {
            Ok(service) => match service.query_status() {
                Ok(s) if s.current_state == ServiceState::Running => State::Running,
                _ => State::Stopped,
            },
            Err(_) => State::NotInstalled,
        }
    }
    #[cfg(not(windows))]
    {
        State::Unsupported
    }
}

/// The signed-in user's SID ("S-1-5-21-…"): only this user may use the pipe.
#[cfg(windows)]
pub fn current_user_sid() -> Result<String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree, HANDLE},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TokenUser, TOKEN_QUERY,
            TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    // SAFETY: plain Win32 calls on this process's token with sized buffers;
    // every handle and allocation is released before returning.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            bail!(
                "Cannot read the user token: {}",
                std::io::Error::last_os_error()
            );
        }
        let mut size = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
        let mut buffer = vec![0u64; (size as usize).div_ceil(8).max(1)];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        );
        CloseHandle(token);
        if ok == 0 {
            bail!("Cannot read the user: {}", std::io::Error::last_os_error());
        }
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut text: windows_sys::core::PWSTR = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
            bail!(
                "Cannot read the user id: {}",
                std::io::Error::last_os_error()
            );
        }
        let len = (0..).take_while(|&i| *text.add(i) != 0).count();
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
        LocalFree(text.cast());
        Ok(sid)
    }
}

/// Runs `noirview-usn.exe` with administrator rights (the Windows prompt)
/// and waits for it; `args` is "--install --user <sid>" or "--uninstall".
fn run_elevated(exe: &Path, args: &str) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE},
            UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
        };
        let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
        let (verb, file, params) = (
            wide("runas".as_ref()),
            wide(exe.as_os_str()),
            wide(args.as_ref()),
        );
        // SAFETY: the strings outlive the call; the process handle is closed.
        unsafe {
            let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
            info.fMask = SEE_MASK_NOCLOSEPROCESS;
            info.lpVerb = verb.as_ptr();
            info.lpFile = file.as_ptr();
            info.lpParameters = params.as_ptr();
            info.nShow = 0;
            if ShellExecuteExW(&mut info) == 0 {
                bail!(
                    "The change was not made ({}).",
                    std::io::Error::last_os_error()
                );
            }
            WaitForSingleObject(info.hProcess, INFINITE);
            let mut code = 0u32;
            GetExitCodeProcess(info.hProcess, &mut code);
            CloseHandle(info.hProcess);
            if code != 0 {
                bail!("noirview-usn.exe {args} failed (exit code {code}); see its log in C:\\ProgramData\\NoirView\\usn.log");
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (exe, args);
        bail!("The USN service is available on Windows only")
    }
}

/// Installs (or updates) the service from the copy next to NoirView.
pub fn install(exe_dir: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let exe = exe_dir.join(protocol::EXE);
        if !exe.exists() {
            bail!("{} is missing from the NoirView folder", protocol::EXE);
        }
        run_elevated(&exe, &format!("--install --user {}", current_user_sid()?))
    }
    #[cfg(not(windows))]
    {
        let _ = exe_dir;
        bail!("The USN service is available on Windows only")
    }
}

pub fn uninstall(exe_dir: &Path) -> Result<()> {
    let exe = exe_dir.join(protocol::EXE);
    if !exe.exists() {
        bail!("{} is missing from the NoirView folder", protocol::EXE);
    }
    run_elevated(&exe, "--uninstall")
}
