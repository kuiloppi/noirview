//! Messages between NoirView and the USN broker service (`noirview-usn.exe`),
//! one JSON object per line over a local named pipe. Shared by both programs.
use serde::{Deserialize, Serialize};

pub const PIPE: &str = r"\\.\pipe\NoirViewUsn";
pub const SERVICE: &str = "NoirViewUsn";
#[allow(dead_code)] // used by the service
pub const DISPLAY_NAME: &str = "NoirView USN broker";
pub const EXE: &str = "noirview-usn.exe";
/// Raised when a message changes incompatibly.
pub const PROTOCOL: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Hello,
    /// The change journal of a volume ("C:").
    Journal {
        volume: String,
    },
    /// Journal facts plus how fast its records can be read.
    Test {
        volume: String,
    },
    /// Changes under `roots` since `from_usn` of journal `journal_id`. A
    /// root the caller cannot list itself is refused.
    Read {
        volume: String,
        journal_id: u64,
        from_usn: i64,
        roots: Vec<String>,
        max: usize,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Journal {
    pub volume: String,
    pub journal_id: u64,
    pub first_usn: i64,
    pub next_usn: i64,
    pub max_size: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Change {
    pub path: String,
    /// `USN_REASON_*` bits, combined over the records read.
    pub reasons: u32,
    pub dir: bool,
    /// Deleted or renamed away, and not there now.
    pub removed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Response {
    Hello {
        version: String,
        protocol: u32,
    },
    Journal(Journal),
    Test {
        journal: Journal,
        records: u64,
        ms: u64,
    },
    Read {
        next_usn: i64,
        changes: Vec<Change>,
        /// More records remain after `next_usn`.
        more: bool,
        /// The journal was recreated or has wrapped past `from_usn`: the
        /// caller must check its folders in full.
        reset: bool,
    },
    Error {
        message: String,
    },
}

/// The drive of a local path ("D:" for `D:\Photos`), if it has one.
pub fn volume_of(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        .then(|| format!("{}:", (bytes[0] as char).to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_as_single_json_lines() {
        let requests = [
            Request::Hello,
            Request::Test {
                volume: "D:".into(),
            },
            Request::Read {
                volume: "D:".into(),
                journal_id: 7,
                from_usn: 1024,
                roots: vec![r"D:\Photos".into()],
                max: 100,
            },
        ];
        for request in requests {
            let line = serde_json::to_string(&request).unwrap();
            assert!(!line.contains('\n'));
            assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), request);
        }
        let response = Response::Read {
            next_usn: 2048,
            changes: vec![Change {
                path: r"D:\Photos\a.jpg".into(),
                reasons: 0x100,
                dir: false,
                removed: false,
            }],
            more: false,
            reset: false,
        };
        let line = serde_json::to_string(&response).unwrap();
        assert_eq!(serde_json::from_str::<Response>(&line).unwrap(), response);
        let journal = Response::Journal(Journal {
            volume: "C:".into(),
            journal_id: 1,
            ..Default::default()
        });
        let line = serde_json::to_string(&journal).unwrap();
        assert!(line.contains(r#""result":"journal""#), "{line}");
        assert_eq!(serde_json::from_str::<Response>(&line).unwrap(), journal);
    }

    #[test]
    fn volume_is_the_drive_letter() {
        assert_eq!(volume_of(r"d:\Photos"), Some("D:".into()));
        assert_eq!(volume_of(r"\\server\share\x"), None);
        assert_eq!(volume_of("/home/x"), None);
    }
}
