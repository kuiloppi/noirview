//! The real `noirview --watch` process: fills the caches of a watched folder,
//! follows new files, refuses a second instance and stops on request.
use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn jpeg(path: &Path, shade: u8) {
    image::RgbImage::from_pixel(64, 48, image::Rgb([shade, 120, 60]))
        .save(path)
        .unwrap();
}
fn status(control: &Path) -> serde_json::Value {
    std::fs::read(control.join("status.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::Value::Null)
}
fn wait(control: &Path, what: &str, until: impl Fn(&serde_json::Value) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let s = status(control);
        if until(&s) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: {s}");
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn watcher(control: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_noirview"));
    command
        .arg("--watch")
        .env("NOIRVIEW_WATCH_CONTROL", control)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[test]
fn background_process_watches_refuses_a_second_copy_and_stops_on_request() {
    let tmp = tempfile::tempdir().unwrap();
    let (control, data, photos) = (
        tmp.path().join("control"),
        tmp.path().join("data"),
        tmp.path().join("photos"),
    );
    std::fs::create_dir_all(&control).unwrap();
    std::fs::create_dir_all(photos.join("sub")).unwrap();
    jpeg(&photos.join("a.jpg"), 10);
    jpeg(&photos.join("sub/b.jpg"), 200);
    let config = serde_json::json!({
        "version": "test",
        "exe": env!("CARGO_BIN_EXE_noirview"),
        "folders": [{"path": photos, "subfolders": true}],
        "settings": {"watch_folders": true, "watch_background": true},
        "cache_root": data.join("thumbnails"),
        "keyword_db": data.join("keyword-index.sqlite3"),
        "state_db": data.join("watch-state.sqlite3"),
        "background": true
    });
    std::fs::write(control.join("config.json"), config.to_string()).unwrap();
    let mut first = watcher(&control).spawn().unwrap();
    wait(&control, "first pass", |s| {
        s["state"] == "watching" && s["thumbnails"] == 2 && s["keywords"] == 2
    });
    // One owner: a second process exits at once.
    let second = watcher(&control).status().unwrap();
    assert!(second.success());
    assert!(first.try_wait().unwrap().is_none(), "first keeps running");
    // A new image is picked up live.
    jpeg(&photos.join("sub/new.jpg"), 90);
    wait(&control, "live file", |s| s["thumbnails"] == 3);
    // Stop request: the process exits and says so.
    std::fs::write(
        control.join("stop"),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let code = loop {
        if let Some(code) = first.try_wait().unwrap() {
            break code;
        }
        assert!(Instant::now() < deadline, "did not stop");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(code.success());
    assert_eq!(status(&control)["state"], "stopped");
    let log = std::fs::read_to_string(control.join("watcher.log")).unwrap();
    assert!(
        log.contains("background test started: 1 folder(s)"),
        "{log}"
    );
    assert!(log.contains("another owner is watching"), "{log}");
    assert!(
        log.contains("background stopped: thumbnails made 3"),
        "{log}"
    );
    // The thumbnails really are in the cache.
    assert!(data.join("thumbnails").exists());
}

/// Needs the USN service installed (CI installs it before running this with
/// `--ignored`): a second start finds a new image through the change journal
/// instead of listing the folder.
#[test]
#[ignore]
fn background_process_catches_up_through_the_change_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let (control, data, photos) = (
        tmp.path().join("control"),
        tmp.path().join("data"),
        tmp.path().join("photos"),
    );
    std::fs::create_dir_all(&control).unwrap();
    std::fs::create_dir_all(&photos).unwrap();
    jpeg(&photos.join("a.jpg"), 10);
    let config = serde_json::json!({
        "version": "test",
        "exe": env!("CARGO_BIN_EXE_noirview"),
        "folders": [{"path": photos, "subfolders": true}],
        "settings": {"watch_folders": true, "watch_background": true, "watch_use_journal": true},
        "cache_root": data.join("thumbnails"),
        "keyword_db": data.join("keyword-index.sqlite3"),
        "state_db": data.join("watch-state.sqlite3"),
        "background": true
    });
    std::fs::write(control.join("config.json"), config.to_string()).unwrap();
    let stop = |control: &Path| {
        std::fs::write(
            control.join("stop"),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .to_string(),
        )
        .unwrap();
    };
    let finish = |child: &mut std::process::Child| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "did not stop");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    // First start: the folder is listed and the journal position stored.
    let mut first = watcher(&control).spawn().unwrap();
    wait(&control, "first pass", |s| {
        s["state"] == "watching" && s["thumbnails"] == 1
    });
    let check = status(&control)["last_check"].clone();
    assert_eq!(check["journal"], 0, "{check}");
    // Let an idle pass store the position, then stop.
    std::thread::sleep(Duration::from_secs(2));
    stop(&control);
    finish(&mut first);
    std::fs::remove_file(control.join("stop")).unwrap();
    // Added while nothing ran.
    jpeg(&photos.join("offline.jpg"), 90);
    let mut second = watcher(&control).spawn().unwrap();
    wait(&control, "journal pass", |s| {
        s["state"] == "watching" && s["thumbnails"] == 1
    });
    let check = status(&control)["last_check"].clone();
    assert_eq!(check["journal"], 1, "{check}");
    assert_eq!(check["folders"], 0, "{check}");
    stop(&control);
    finish(&mut second);
    let log = std::fs::read_to_string(control.join("watcher.log")).unwrap();
    assert!(log.contains("using the change journal"), "{log}");
}
