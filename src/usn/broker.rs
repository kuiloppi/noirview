//! The USN broker service (Windows). It runs as LocalSystem because only an
//! administrator may open a volume and read its change journal, and it does
//! nothing else: no image is opened or decoded here. NoirView asks over a
//! local named pipe that only the installing user (and SYSTEM) may open; a
//! folder is answered only if that user can list it (checked while
//! impersonating the caller).
use crate::protocol::{self, Change, Journal, Request, Response};
use std::{
    collections::HashMap,
    ffi::OsString,
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::FromRawHandle,
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, LocalFree, ERROR_HANDLE_EOF, ERROR_PIPE_CONNECTED, HANDLE,
        INVALID_HANDLE_VALUE,
    },
    Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        RevertToSelf, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{
        CreateFileW, FileIdType, GetFinalPathNameByHandleW, OpenFileById,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_ID_DESCRIPTOR,
        FILE_LIST_DIRECTORY, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, VOLUME_NAME_DOS,
    },
    System::{
        Ioctl::{
            FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL, READ_USN_JOURNAL_DATA_V0,
            USN_JOURNAL_DATA_V0,
        },
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, ImpersonateNamedPipeClient,
            PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
            PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        },
        IO::DeviceIoControl,
    },
};

const GENERIC_READ: u32 = 0x8000_0000;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub const REASON_FILE_DELETE: u32 = 0x200;
pub const REASON_RENAME_OLD_NAME: u32 = 0x1000;
/// Paths of parent folders remembered while answering (cleared beyond this).
const PARENT_CACHE: usize = 200_000;

fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
fn last_error() -> io::Error {
    io::Error::last_os_error()
}

/// `C:\ProgramData\NoirView`: the service's log (readable by users).
pub fn data_dir() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("NoirView")
}
pub fn log(line: &str) {
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("usn.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1 << 20) {
        let _ = std::fs::rename(&path, dir.join("usn.old.log"));
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(
            f,
            "{:?} [{}] {line}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            std::process::id()
        );
    }
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is owned and closed once.
        unsafe { CloseHandle(self.0) };
    }
}

/// An open volume ("C:") and its change journal.
pub struct Volume {
    handle: Handle,
    pub name: String,
    parents: HashMap<u64, Option<String>>,
}
impl Volume {
    pub fn open(name: &str) -> io::Result<Self> {
        let name = protocol::volume_of(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a drive letter"))?;
        let path = wide(format!(r"\\.\{name}"));
        // SAFETY: a valid NUL-terminated path; the handle is owned by `Handle`.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(last_error());
        }
        Ok(Self {
            handle: Handle(handle),
            name,
            parents: HashMap::new(),
        })
    }
    pub fn journal(&self) -> io::Result<Journal> {
        let mut data: USN_JOURNAL_DATA_V0 = unsafe { std::mem::zeroed() };
        let mut returned = 0u32;
        // SAFETY: output buffer of the documented type and size.
        let ok = unsafe {
            DeviceIoControl(
                self.handle.0,
                FSCTL_QUERY_USN_JOURNAL,
                std::ptr::null(),
                0,
                (&mut data as *mut USN_JOURNAL_DATA_V0).cast(),
                std::mem::size_of::<USN_JOURNAL_DATA_V0>() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(last_error());
        }
        Ok(Journal {
            volume: self.name.clone(),
            journal_id: data.UsnJournalID,
            first_usn: data.FirstUsn,
            next_usn: data.NextUsn,
            max_size: data.MaximumSize,
        })
    }
    /// Calls `each` for every record from `from` up to the journal end seen
    /// at the start (or until `each` returns false). Returns the next USN.
    pub fn records(
        &self,
        journal_id: u64,
        from: i64,
        until: i64,
        mut each: impl FnMut(Record) -> bool,
    ) -> io::Result<i64> {
        let mut request = READ_USN_JOURNAL_DATA_V0 {
            StartUsn: from,
            ReasonMask: u32::MAX,
            ReturnOnlyOnClose: 0,
            Timeout: 0,
            BytesToWaitFor: 0,
            UsnJournalID: journal_id,
        };
        let mut buffer = vec![0u64; 64 * 1024 / 8];
        let bytes = buffer.len() * 8;
        let mut next = from;
        while next < until {
            let mut returned = 0u32;
            // SAFETY: input and output buffers of the documented sizes.
            let ok = unsafe {
                DeviceIoControl(
                    self.handle.0,
                    FSCTL_READ_USN_JOURNAL,
                    (&request as *const READ_USN_JOURNAL_DATA_V0).cast(),
                    std::mem::size_of::<READ_USN_JOURNAL_DATA_V0>() as u32,
                    buffer.as_mut_ptr().cast(),
                    bytes as u32,
                    &mut returned,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                // SAFETY: reads the calling thread's last error.
                if unsafe { GetLastError() } == ERROR_HANDLE_EOF {
                    break;
                }
                return Err(last_error());
            }
            // SAFETY: the buffer holds `returned` initialized bytes.
            let data = unsafe {
                std::slice::from_raw_parts(buffer.as_ptr() as *const u8, returned as usize)
            };
            if data.len() < 8 {
                break;
            }
            let following = i64::from_le_bytes(data[..8].try_into().unwrap());
            let mut offset = 8;
            let mut any = false;
            while offset + 60 <= data.len() {
                let length =
                    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
                if length < 60 || offset + length > data.len() {
                    break;
                }
                if let Some(record) = Record::parse(&data[offset..offset + length]) {
                    any = true;
                    if record.usn >= until {
                        return Ok(record.usn);
                    }
                    if !each(record) {
                        return Ok(following.min(until));
                    }
                }
                offset += length;
            }
            if !any || following <= next {
                next = following;
                break;
            }
            next = following;
            request.StartUsn = following;
        }
        Ok(next)
    }
    /// The full path of a folder by its file reference number.
    fn folder_path(&mut self, reference: u64) -> Option<String> {
        if let Some(found) = self.parents.get(&reference) {
            return found.clone();
        }
        if self.parents.len() >= PARENT_CACHE {
            self.parents.clear();
        }
        let path = open_by_id(&self.handle, reference).and_then(|h| final_path(&h));
        self.parents.insert(reference, path.clone());
        path
    }
}

/// One USN_RECORD_V2 (NTFS; other versions are skipped).
#[derive(Debug, Clone)]
pub struct Record {
    pub parent: u64,
    pub usn: i64,
    pub reason: u32,
    pub dir: bool,
    pub name: String,
}
impl Record {
    fn parse(raw: &[u8]) -> Option<Self> {
        let u16_at = |o: usize| u16::from_le_bytes(raw[o..o + 2].try_into().unwrap());
        let u32_at = |o: usize| u32::from_le_bytes(raw[o..o + 4].try_into().unwrap());
        let u64_at = |o: usize| u64::from_le_bytes(raw[o..o + 8].try_into().unwrap());
        if u16_at(4) != 2 {
            return None;
        }
        let (name_len, name_off) = (u16_at(56) as usize, u16_at(58) as usize);
        let name = raw.get(name_off..name_off + name_len)?;
        let units: Vec<u16> = name
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        Some(Self {
            parent: u64_at(16),
            usn: u64_at(24) as i64,
            reason: u32_at(40),
            dir: u32_at(52) & FILE_ATTRIBUTE_DIRECTORY != 0,
            name: OsString::from_wide(&units).to_string_lossy().into_owned(),
        })
    }
}

fn open_by_id(volume: &Handle, reference: u64) -> Option<Handle> {
    let mut id: FILE_ID_DESCRIPTOR = unsafe { std::mem::zeroed() };
    id.dwSize = std::mem::size_of::<FILE_ID_DESCRIPTOR>() as u32;
    id.Type = FileIdType;
    id.Anonymous.FileId = reference as i64;
    // SAFETY: a valid descriptor; the handle is owned by `Handle`.
    let handle = unsafe {
        OpenFileById(
            volume.0,
            &id,
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            FILE_FLAG_BACKUP_SEMANTICS,
        )
    };
    (handle != INVALID_HANDLE_VALUE).then_some(Handle(handle))
}
/// `C:\Photos` for an open handle (without the `\\?\` prefix).
fn final_path(handle: &Handle) -> Option<String> {
    let mut buffer = vec![0u16; 1024];
    loop {
        // SAFETY: the buffer length is passed in characters.
        let len = unsafe {
            GetFinalPathNameByHandleW(
                handle.0,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        } as usize;
        if len == 0 {
            return None;
        }
        if len >= buffer.len() {
            buffer.resize(len + 1, 0);
            continue;
        }
        let path = String::from_utf16_lossy(&buffer[..len]);
        return Some(match path.strip_prefix(r"\\?\UNC\") {
            Some(rest) => format!(r"\\{rest}"),
            None => path.strip_prefix(r"\\?\").unwrap_or(&path).to_string(),
        });
    }
}
/// A folder as the caller sees it: opened while impersonating the pipe
/// client, so a folder the user may not list is refused.
fn caller_folder(pipe: HANDLE, path: &str) -> Option<String> {
    let path = wide(path);
    // SAFETY: impersonation is reverted before returning.
    unsafe {
        if ImpersonateNamedPipeClient(pipe) == 0 {
            return None;
        }
        let handle = CreateFileW(
            path.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        );
        let opened = (handle != INVALID_HANDLE_VALUE).then_some(Handle(handle));
        let canonical = opened.as_ref().and_then(final_path);
        drop(opened);
        RevertToSelf();
        canonical
    }
}

fn lower_dir(path: &str) -> String {
    let mut p = path.to_lowercase();
    if !p.ends_with('\\') {
        p.push('\\');
    }
    p
}

/// Changes under `roots` (canonical folders) since `from`.
pub fn read_changes(
    volume: &mut Volume,
    journal_id: u64,
    from: i64,
    roots: &[String],
    max: usize,
) -> io::Result<Response> {
    let journal = volume.journal()?;
    if journal.journal_id != journal_id || from < journal.first_usn || from > journal.next_usn {
        return Ok(Response::Read {
            next_usn: journal.next_usn,
            changes: vec![],
            more: false,
            reset: true,
        });
    }
    let roots: Vec<String> = roots.iter().map(|r| lower_dir(r)).collect();
    let mut found: Vec<(String, u32, bool)> = vec![];
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut records = vec![];
    let next = volume.records(journal_id, from, journal.next_usn, |r| {
        records.push(r);
        records.len() < 50_000
    })?;
    let mut more = next < journal.next_usn;
    for record in records {
        let Some(folder) = volume.folder_path(record.parent) else {
            continue;
        };
        let path = format!(r"{}\{}", folder.trim_end_matches('\\'), record.name);
        let lower = path.to_lowercase();
        if !roots.iter().any(|r| lower.starts_with(r.as_str())) {
            continue;
        }
        match index.get(&lower) {
            Some(&i) => found[i].1 |= record.reason,
            None => {
                if found.len() >= max {
                    more = true;
                    break;
                }
                index.insert(lower, found.len());
                found.push((path, record.reason, record.dir));
            }
        }
    }
    let changes = found
        .into_iter()
        .map(|(path, reasons, dir)| Change {
            removed: reasons & (REASON_FILE_DELETE | REASON_RENAME_OLD_NAME) != 0
                && !Path::new(&path).exists(),
            path,
            reasons,
            dir,
        })
        .collect();
    Ok(Response::Read {
        next_usn: next,
        changes,
        more,
        reset: false,
    })
}

/// How fast the whole journal can be read (no paths resolved).
pub fn test(volume: &Volume) -> io::Result<Response> {
    let journal = volume.journal()?;
    let started = Instant::now();
    let mut records = 0u64;
    volume.records(
        journal.journal_id,
        journal.first_usn,
        journal.next_usn,
        |_| {
            records += 1;
            true
        },
    )?;
    Ok(Response::Test {
        journal,
        records,
        ms: started.elapsed().as_millis() as u64,
    })
}

fn volume<'a>(volumes: &'a mut HashMap<String, Volume>, name: &str) -> io::Result<&'a mut Volume> {
    let key = protocol::volume_of(name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a drive letter"))?;
    if !volumes.contains_key(&key) {
        let opened = Volume::open(&key)?;
        volumes.insert(key.clone(), opened);
    }
    Ok(volumes.get_mut(&key).unwrap())
}

fn answer(pipe: HANDLE, request: Request, volumes: &mut HashMap<String, Volume>) -> Response {
    let result = match request {
        Request::Hello => Ok(Response::Hello {
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: protocol::PROTOCOL,
        }),
        Request::Journal { volume: name } => volume(volumes, &name)
            .and_then(|v| v.journal())
            .map(Response::Journal),
        Request::Test { volume: name } => volume(volumes, &name).and_then(|v| test(v)),
        Request::Read {
            volume: name,
            journal_id,
            from_usn,
            roots,
            max,
        } => {
            let mut allowed = vec![];
            for root in &roots {
                match caller_folder(pipe, root) {
                    Some(canonical) => allowed.push(canonical),
                    None => {
                        return Response::Error {
                            message: format!("{root}: not accessible to the caller"),
                        }
                    }
                }
            }
            volume(volumes, &name)
                .and_then(|v| read_changes(v, journal_id, from_usn, &allowed, max.max(1)))
        }
    };
    result.unwrap_or_else(|e| Response::Error {
        message: e.to_string(),
    })
}

fn serve(pipe: HANDLE) {
    // SAFETY: the connected pipe handle is owned by the File from here on.
    let file = unsafe { File::from_raw_handle(pipe as _) };
    let mut volumes = HashMap::new();
    let mut reader = BufReader::new(&file);
    let mut line = String::new();
    while reader.read_line(&mut line).is_ok_and(|n| n > 0) {
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => answer(pipe, request, &mut volumes),
            Err(e) => Response::Error {
                message: format!("bad request: {e}"),
            },
        };
        line.clear();
        let mut text = serde_json::to_string(&response).unwrap_or_default();
        text.push('\n');
        if (&file).write_all(text.as_bytes()).is_err() {
            break;
        }
    }
    // SAFETY: the handle is still open (owned by `file`).
    unsafe { DisconnectNamedPipe(pipe) };
}

/// Pipe security: full access for SYSTEM and read/write for one user.
struct Security {
    descriptor: PSECURITY_DESCRIPTOR,
}
impl Security {
    fn for_user(sid: &str) -> io::Result<Self> {
        if !sid.starts_with("S-1-") || !sid[4..].chars().all(|c| c.is_ascii_digit() || c == '-') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad user id"));
        }
        let sddl = wide(format!("D:P(A;;GA;;;SY)(A;;GRGW;;;{sid})"));
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: a valid SDDL string; the descriptor is freed in Drop.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(last_error());
        }
        Ok(Self { descriptor })
    }
}
impl Drop for Security {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe { LocalFree(self.descriptor) };
    }
}

/// Accepts clients on `name` until `stop` is set (then connect once to wake it).
pub fn run_server(name: &str, user_sid: &str, stop: Arc<AtomicBool>) -> io::Result<()> {
    let security = Security::for_user(user_sid)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security.descriptor,
        bInheritHandle: 0,
    };
    let name = wide(name);
    let mut first = true;
    while !stop.load(Ordering::Relaxed) {
        // The first instance claims the name, so no other process can
        // squat on the pipe and impersonate the service.
        let mode = PIPE_ACCESS_DUPLEX
            | if first {
                FILE_FLAG_FIRST_PIPE_INSTANCE
            } else {
                0
            };
        // SAFETY: valid name and security attributes that outlive the call.
        let pipe = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                &attributes,
            )
        };
        if pipe == INVALID_HANDLE_VALUE {
            let e = last_error();
            if first {
                return Err(e);
            }
            log(&format!("pipe: {e}"));
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }
        first = false;
        // SAFETY: a valid pipe handle; blocks until a client connects.
        let connected = unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) } != 0
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if stop.load(Ordering::Relaxed) || !connected {
            // SAFETY: closing the unused instance.
            unsafe { CloseHandle(pipe) };
            continue;
        }
        let pipe = pipe as usize;
        std::thread::spawn(move || serve(pipe as HANDLE));
    }
    Ok(())
}

/// Wakes a server blocked in ConnectNamedPipe.
pub fn poke(name: &str) {
    let _ = OpenOptions::new().read(true).write(true).open(name);
}

/// A client for `--probe` and the tests.
pub fn ask(name: &str, request: &Request) -> io::Result<Response> {
    let pipe = OpenOptions::new().read(true).write(true).open(name)?;
    let mut line = serde_json::to_string(request).map_err(io::Error::other)?;
    line.push('\n');
    (&pipe).write_all(line.as_bytes())?;
    let mut answer = String::new();
    BufReader::new(&pipe).read_line(&mut answer)?;
    serde_json::from_str(&answer).map_err(io::Error::other)
}

pub mod service {
    //! Running as a Windows service, and installing / removing it.
    use super::*;
    use windows_service::{
        define_windows_service,
        service::{
            ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl,
            ServiceExitCode, ServiceInfo, ServiceStartType, ServiceState, ServiceStatus,
            ServiceType,
        },
        service_control_handler::{self, ServiceControlHandlerResult},
        service_dispatcher,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    define_windows_service!(ffi_service_main, service_main);

    /// Called by `main` when the service manager starts the program.
    pub fn dispatch() -> windows_service::Result<()> {
        service_dispatcher::start(protocol::SERVICE, ffi_service_main)
    }

    fn user_from(arguments: &[OsString]) -> Option<String> {
        let args: Vec<String> = arguments
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        args.iter()
            .position(|a| a == "--user")
            .and_then(|i| args.get(i + 1).cloned())
    }

    fn service_main(_arguments: Vec<OsString>) {
        // The user comes from the launch arguments fixed at installation.
        let launch: Vec<OsString> = std::env::args_os().collect();
        let Some(user) = user_from(&launch) else {
            log("service: no --user in the launch arguments");
            return;
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let handler = move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                stopping.store(true, Ordering::Relaxed);
                poke(protocol::PIPE);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let Ok(status) = service_control_handler::register(protocol::SERVICE, handler) else {
            return;
        };
        let report = |state, accept| {
            let _ = status.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: state,
                controls_accepted: accept,
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: Duration::from_secs(5),
                process_id: None,
            });
        };
        report(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        );
        log(&format!("service {} started", env!("CARGO_PKG_VERSION")));
        if let Err(e) = run_server(protocol::PIPE, &user, stop) {
            log(&format!("service: {e}"));
        }
        log("service stopped");
        report(ServiceState::Stopped, ServiceControlAccept::empty());
    }

    /// The protected copy the service runs (only administrators may change it).
    fn installed_exe() -> PathBuf {
        std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
            .join("NoirView")
            .join(protocol::EXE)
    }

    fn stop_and_delete(manager: &ServiceManager) -> windows_service::Result<bool> {
        let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
        let service = match manager.open_service(protocol::SERVICE, access) {
            Ok(s) => s,
            Err(_) => return Ok(false),
        };
        if service.query_status()?.current_state != ServiceState::Stopped {
            let _ = service.stop();
            let deadline = Instant::now() + Duration::from_secs(15);
            while service.query_status()?.current_state != ServiceState::Stopped
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        service.delete()?;
        drop(service);
        // The manager removes it once every handle is closed.
        let deadline = Instant::now() + Duration::from_secs(10);
        while manager
            .open_service(protocol::SERVICE, ServiceAccess::QUERY_STATUS)
            .is_ok()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(true)
    }

    /// `--install --user <sid>` (administrator): replaces any earlier copy.
    pub fn install(user: &str) -> Result<(), String> {
        Security::for_user(user).map_err(|e| format!("user id: {e}"))?;
        let manager = ServiceManager::local_computer(
            None::<&str>,
            ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
        )
        .map_err(|e| format!("service manager (administrator rights needed): {e}"))?;
        stop_and_delete(&manager).map_err(|e| format!("removing the old service: {e}"))?;
        let target = installed_exe();
        std::fs::create_dir_all(target.parent().unwrap())
            .map_err(|e| format!("{}: {e}", target.display()))?;
        let source = std::env::current_exe().map_err(|e| e.to_string())?;
        if source != target {
            std::fs::copy(&source, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        }
        let info = ServiceInfo {
            name: protocol::SERVICE.into(),
            display_name: protocol::DISPLAY_NAME.into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: target.clone(),
            launch_arguments: vec!["--service".into(), "--user".into(), user.into()],
            dependencies: vec![],
            account_name: None,
            account_password: None,
        };
        let service = manager
            .create_service(
                &info,
                ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::QUERY_STATUS,
            )
            .map_err(|e| format!("creating the service: {e}"))?;
        let _ = service.set_description(
            "Reads the NTFS change journal for NoirView's folder watcher. Opens no images.",
        );
        let _ = service.set_delayed_auto_start(true);
        restrict_privileges();
        service
            .start::<&str>(&[])
            .map_err(|e| format!("starting the service: {e}"))?;
        log(&format!(
            "installed {} for {user} at {}",
            env!("CARGO_PKG_VERSION"),
            target.display()
        ));
        Ok(())
    }

    /// Only the privileges the service uses: traverse checks, impersonating
    /// the pipe client, and opening folders by id.
    fn restrict_privileges() {
        use windows_sys::Win32::System::Services::{
            ChangeServiceConfig2W, CloseServiceHandle, OpenSCManagerW, OpenServiceW,
            SC_MANAGER_CONNECT, SERVICE_CHANGE_CONFIG, SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO,
            SERVICE_REQUIRED_PRIVILEGES_INFOW,
        };
        let mut list: Vec<u16> = [
            "SeChangeNotifyPrivilege",
            "SeImpersonatePrivilege",
            "SeBackupPrivilege",
        ]
        .iter()
        .flat_map(|p| p.encode_utf16().chain(Some(0)))
        .chain(Some(0))
        .collect();
        let name = wide(protocol::SERVICE);
        // SAFETY: handles are closed; the list is a valid double-NUL string.
        unsafe {
            let manager = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
            if manager.is_null() {
                return;
            }
            let service = OpenServiceW(manager, name.as_ptr(), SERVICE_CHANGE_CONFIG);
            if !service.is_null() {
                let info = SERVICE_REQUIRED_PRIVILEGES_INFOW {
                    pmszRequiredPrivileges: list.as_mut_ptr(),
                };
                if ChangeServiceConfig2W(
                    service,
                    SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO,
                    (&info as *const SERVICE_REQUIRED_PRIVILEGES_INFOW).cast(),
                ) == 0
                {
                    log(&format!("privileges: {}", last_error()));
                }
                CloseServiceHandle(service);
            }
            CloseServiceHandle(manager);
        }
    }

    /// `--uninstall` (administrator).
    pub fn uninstall() -> Result<(), String> {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .map_err(|e| format!("service manager (administrator rights needed): {e}"))?;
        let removed =
            stop_and_delete(&manager).map_err(|e| format!("removing the service: {e}"))?;
        let target = installed_exe();
        let current = std::env::current_exe().ok();
        if current.as_deref() != Some(target.as_path()) {
            let _ = std::fs::remove_file(&target);
            if let Some(dir) = target.parent() {
                let _ = std::fs::remove_dir(dir);
            }
        }
        log(if removed {
            "uninstalled"
        } else {
            "uninstall: not installed"
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> String {
        // The current user's SID from `whoami /user` (CSV: name,sid).
        let out = std::process::Command::new("whoami")
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        text.trim()
            .rsplit(',')
            .next()
            .unwrap()
            .trim_matches('"')
            .to_string()
    }

    /// CI runs as an administrator, so the journal of the temp drive can be
    /// read; elsewhere the test only checks that access is refused cleanly.
    #[test]
    fn a_new_file_appears_in_the_journal_under_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let root = root
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_string();
        let mut volume = match Volume::open(&root) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("volume not readable here ({e}); skipped");
                return;
            }
        };
        let before = volume.journal().unwrap();
        std::fs::write(dir.path().join("moss.jpg"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("deep.png"), b"y").unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("other.jpg"), b"z").unwrap();
        let response = read_changes(
            &mut volume,
            before.journal_id,
            before.next_usn,
            std::slice::from_ref(&root),
            1000,
        )
        .unwrap();
        let Response::Read { changes, reset, .. } = response else {
            panic!("{response:?}")
        };
        assert!(!reset);
        let names: Vec<String> = changes
            .iter()
            .map(|c| {
                Path::new(&c.path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(names.contains(&"moss.jpg".to_string()), "{names:?}");
        assert!(names.contains(&"deep.png".to_string()), "{names:?}");
        assert!(!names.contains(&"other.jpg".to_string()), "{names:?}");
        assert!(changes.iter().any(|c| c.dir && c.path.ends_with("sub")));
        // A deleted file is reported as removed.
        std::fs::remove_file(dir.path().join("moss.jpg")).unwrap();
        let after = volume.journal().unwrap();
        let Response::Read { changes, .. } = read_changes(
            &mut volume,
            after.journal_id,
            before.next_usn,
            std::slice::from_ref(&root),
            1000,
        )
        .unwrap() else {
            panic!()
        };
        assert!(changes
            .iter()
            .any(|c| c.path.ends_with("moss.jpg") && c.removed));
        // A wrong journal id or a position before the journal start: reset.
        let Response::Read { reset, .. } = read_changes(
            &mut volume,
            after.journal_id + 1,
            after.next_usn,
            std::slice::from_ref(&root),
            10,
        )
        .unwrap() else {
            panic!()
        };
        assert!(reset);
        // Reading speed for Settings → Test.
        let Response::Test { records, .. } = test(&volume).unwrap() else {
            panic!()
        };
        assert!(records > 0);
    }

    #[test]
    fn the_pipe_answers_its_user_and_checks_the_folders() {
        let name = format!(r"\\.\pipe\NoirViewUsnTest{}", std::process::id());
        let stop = Arc::new(AtomicBool::new(false));
        let (server_name, server_stop, sid) = (name.clone(), stop.clone(), user());
        let server = std::thread::spawn(move || run_server(&server_name, &sid, server_stop));
        let deadline = Instant::now() + Duration::from_secs(10);
        let hello = loop {
            match ask(&name, &Request::Hello) {
                Ok(r) => break r,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Err(e) => panic!("{e}"),
            }
        };
        assert_eq!(
            hello,
            Response::Hello {
                version: env!("CARGO_PKG_VERSION").into(),
                protocol: protocol::PROTOCOL
            }
        );
        // A folder that does not exist is refused before anything is read.
        let refused = ask(
            &name,
            &Request::Read {
                volume: "C:".into(),
                journal_id: 0,
                from_usn: 0,
                roots: vec![r"C:\no\such\folder".into()],
                max: 10,
            },
        )
        .unwrap();
        assert!(matches!(refused, Response::Error { .. }), "{refused:?}");
        // Another user's SID: the pipe cannot be opened by this user.
        let other = format!(r"\\.\pipe\NoirViewUsnOther{}", std::process::id());
        let other_stop = Arc::new(AtomicBool::new(false));
        let (o_name, o_stop) = (other.clone(), other_stop.clone());
        let other_server =
            std::thread::spawn(move || run_server(&o_name, "S-1-5-21-1-2-3-1001", o_stop));
        std::thread::sleep(Duration::from_millis(300));
        let denied = ask(&other, &Request::Hello);
        assert!(denied.is_err(), "{denied:?}");
        other_stop.store(true, Ordering::Relaxed);
        stop.store(true, Ordering::Relaxed);
        poke(&name);
        // The other server cannot be poked by us (access denied); it ends
        // with the test process.
        drop(other_server);
        server.join().unwrap().unwrap();
        // Malformed user ids are rejected.
        assert!(Security::for_user("S-1-5;(A;;GA;;;WD)").is_err());
    }
}
