//! Opens a file dialog where it was last left (Windows).
//!
//! Windows keeps a file dialog's size and last folder under its client GUID
//! but centres a dialog owned by a window over it, so the place is kept
//! here: a window-event hook on the dialog's own thread moves the dialog as
//! it is shown and follows it until it closes.

/// A dialog's top-left corner in screen pixels.
pub type Place = [i32; 2];

/// Runs `show`, which opens a file dialog on this thread and returns once
/// it closes, with the dialog moved to `place` (when that is still on a
/// screen). Returns `show`'s result and where the dialog was at the end.
#[cfg(windows)]
pub fn track<T>(place: Option<Place>, show: impl FnOnce() -> T) -> (T, Option<Place>) {
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_SHOW, WINEVENT_OUTOFCONTEXT,
    };
    imp::STATE.with(|s| {
        *s.borrow_mut() = imp::Tracked {
            want: place,
            ..Default::default()
        }
    });
    // Events of this thread only; they arrive while the dialog's modal loop
    // pumps messages.
    let hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_SHOW,
            EVENT_OBJECT_LOCATIONCHANGE,
            std::ptr::null_mut(),
            Some(imp::on_event),
            std::process::id(),
            windows_sys::Win32::System::Threading::GetCurrentThreadId(),
            WINEVENT_OUTOFCONTEXT,
        )
    };
    let result = show();
    if !hook.is_null() {
        unsafe { UnhookWinEvent(hook) };
    }
    let last = imp::STATE.with(|s| std::mem::take(&mut *s.borrow_mut()).last);
    (result, last)
}

#[cfg(not(windows))]
pub fn track<T>(_place: Option<Place>, show: impl FnOnce() -> T) -> (T, Option<Place>) {
    (show(), None)
}

#[cfg(windows)]
mod imp {
    use super::Place;
    use std::cell::RefCell;
    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::Graphics::Gdi::{MonitorFromRect, MONITOR_DEFAULTTONULL};
    use windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetClassNameW, GetWindowRect, SetWindowPos, EVENT_OBJECT_LOCATIONCHANGE,
        EVENT_OBJECT_SHOW, GA_ROOT, OBJID_WINDOW, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    };

    #[derive(Default)]
    pub struct Tracked {
        pub want: Option<Place>,
        pub dialog: Option<usize>,
        pub last: Option<Place>,
    }

    thread_local! {
        pub static STATE: RefCell<Tracked> = RefCell::new(Tracked::default());
    }

    /// A top-level dialog window (`#32770`), as the file dialog's frame is.
    fn is_dialog_frame(hwnd: HWND) -> bool {
        if unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd {
            return false;
        }
        let mut name = [0u16; 16];
        let n = unsafe { GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32) };
        n > 0 && String::from_utf16_lossy(&name[..n as usize]) == "#32770"
    }

    fn rect(hwnd: HWND) -> Option<RECT> {
        let mut r = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        (unsafe { GetWindowRect(hwnd, &mut r) } != 0).then_some(r)
    }

    /// The dialog's title bar at `place` would be on a screen.
    fn on_screen(place: Place, size: &RECT) -> bool {
        let width = (size.right - size.left).max(1);
        let bar = RECT {
            left: place[0],
            top: place[1],
            right: place[0] + width.min(400),
            bottom: place[1] + 30,
        };
        !unsafe { MonitorFromRect(&bar, MONITOR_DEFAULTTONULL) }.is_null()
    }

    pub unsafe extern "system" fn on_event(
        _hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        object: i32,
        child: i32,
        _thread: u32,
        _time: u32,
    ) {
        if object != OBJID_WINDOW || child != 0 || hwnd.is_null() {
            return;
        }
        STATE.with(|s| {
            // Moving the dialog below could, in principle, deliver an event
            // while the state is held.
            let Ok(mut s) = s.try_borrow_mut() else {
                return;
            };
            let ours = s.dialog == Some(hwnd as usize);
            if event == EVENT_OBJECT_SHOW && s.dialog.is_none() && is_dialog_frame(hwnd) {
                s.dialog = Some(hwnd as usize);
                if let (Some(want), Some(r)) = (s.want, rect(hwnd)) {
                    if on_screen(want, &r) {
                        unsafe {
                            SetWindowPos(
                                hwnd,
                                std::ptr::null_mut(),
                                want[0],
                                want[1],
                                0,
                                0,
                                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                            )
                        };
                    }
                }
            } else if !(ours && event == EVENT_OBJECT_LOCATIONCHANGE) {
                return;
            }
            if let Some(r) = rect(hwnd) {
                s.last = Some([r.left, r.top]);
            }
        });
    }
}
