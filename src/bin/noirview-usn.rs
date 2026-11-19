//! `noirview-usn.exe`: NoirView's optional NTFS change-journal broker. Runs as
//! a Windows service (LocalSystem) installed from NoirView's Settings with one
//! administrator prompt. It reads the change journal and answers NoirView over
//! a named pipe that only the installing user may open; it opens no images.
//!
//! Usage (administrator for install/uninstall):
//!   noirview-usn.exe --install --user <SID>
//!   noirview-usn.exe --uninstall
//!   noirview-usn.exe --probe [C:]

#[cfg(windows)]
#[path = "../usn/broker.rs"]
mod broker;
#[path = "../usn/protocol.rs"]
#[allow(dead_code)]
mod protocol;

#[cfg(windows)]
fn main() {
    use protocol::{Request, Response};
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let code = if has("--service") {
        match broker::service::dispatch() {
            Ok(()) => 0,
            Err(e) => {
                broker::log(&format!("dispatcher: {e}"));
                1
            }
        }
    } else if has("--install") {
        match value("--user") {
            Some(user) => match broker::service::install(&user) {
                Ok(()) => 0,
                Err(e) => {
                    broker::log(&format!("install failed: {e}"));
                    eprintln!("{e}");
                    1
                }
            },
            None => {
                eprintln!("--install needs --user <SID>");
                2
            }
        }
    } else if has("--uninstall") {
        match broker::service::uninstall() {
            Ok(()) => 0,
            Err(e) => {
                broker::log(&format!("uninstall failed: {e}"));
                eprintln!("{e}");
                1
            }
        }
    } else if has("--probe") {
        let volume = value("--probe").unwrap_or_else(|| "C:".into());
        let mut ok = true;
        for request in [
            Request::Hello,
            Request::Journal {
                volume: volume.clone(),
            },
            Request::Test { volume },
        ] {
            match broker::ask(protocol::PIPE, &request) {
                Ok(Response::Error { message }) => {
                    println!("{request:?}: error: {message}");
                    ok = false;
                }
                Ok(response) => println!("{response:?}"),
                Err(e) => {
                    println!("{request:?}: {e}");
                    ok = false;
                }
            }
        }
        if ok {
            0
        } else {
            1
        }
    } else {
        eprintln!(
            "NoirView USN broker. Install it from NoirView: Settings → Library → Watched folders."
        );
        2
    };
    std::process::exit(code);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("noirview-usn is a Windows service; nothing to do here.");
    std::process::exit(2);
}
