# NoirView's changes to rfd 0.15.4

rfd 0.15.4 from crates.io (MIT, see `LICENSE`), changed as follows. Left out: the changelog, `Cargo.toml.orig`, `deny.toml`, the unused `src/backend/win_xp.rs.old` and the examples (their `[[example]]` entries removed from `Cargo.toml`). To update rfd, copy the new version here and re-apply the changes (search for "NoirView patch").

## Persisted dialog state on Windows

- `FileDialog::set_client_guid(u128)` (`src/file_dialog.rs`, field `client_guid`): other platforms ignore it.
- `src/backend/win_cid/file_dialog/dialog_ffi.rs`: `DialogInner::set_client_guid` calls `IFileDialog::SetClientGuid` right after the dialog is created (open and save dialogs), before `Show`. Windows then keeps the dialog's position, size and last folder under that GUID instead of sharing one state for the whole program.
- `src/backend/win_cid/file_dialog/com.rs`: the vtable's `SetClientGuid` entry is visible to `dialog_ffi.rs`.
