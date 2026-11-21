//! Opens a file dialog where it was last left (Windows).
//!
//! Windows keeps a file dialog's size and last folder under its client GUID
//! but centres a dialog owned by a window over it. So the dialog's frame
//! window is caught as it is created (a CBT hook on the dialog's own
//! thread) and its window procedure wrapped: every placement before it is
//! first shown goes to the saved place, so it appears there directly, and
//! each later move is recorded for next time.

/// A dialog's top-left corner in screen pixels.
pub type Place = [i32; 2];

/// Runs `show`, which opens a file dialog on this thread and returns once
/// it closes, with the dialog placed at `place` (when that is still on a
/// screen). Returns `show`'s result and where the dialog was at the end.
#[cfg(windows)]
pub fn track<T>(place: Option<Place>, show: impl FnOnce() -> T) -> (T, Option<Place>) {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowsHookExW, UnhookWindowsHookEx, WH_CBT,
    };
    imp::STATE.with(|s| {
        *s.borrow_mut() = imp::Tracked {
            want: place,
            ..Default::default()
        }
    });
    let hook = unsafe {
        SetWindowsHookExW(
            WH_CBT,
            Some(imp::on_cbt),
            std::ptr::null_mut(),
            GetCurrentThreadId(),
        )
    };
    let result = show();
    if !hook.is_null() {
        unsafe { UnhookWindowsHookEx(hook) };
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
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{MonitorFromRect, MONITOR_DEFAULTTONULL};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, CallWindowProcW, GetClassNameW, GetWindowRect, IsIconic, SetWindowLongPtrW,
        CBT_CREATEWNDW, GWLP_WNDPROC, HCBT_CREATEWND, SWP_NOMOVE, SWP_SHOWWINDOW, WINDOWPOS,
        WM_NCDESTROY, WM_WINDOWPOSCHANGED, WM_WINDOWPOSCHANGING, WS_CHILD,
    };

    type Proc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

    #[derive(Default)]
    pub struct Tracked {
        pub want: Option<Place>,
        /// The dialog's frame window and its own window procedure.
        pub dialog: Option<(usize, isize)>,
        pub shown: bool,
        pub last: Option<Place>,
    }

    thread_local! {
        pub static STATE: RefCell<Tracked> = RefCell::new(Tracked::default());
    }

    fn class_is_dialog(hwnd: HWND) -> bool {
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
    fn on_screen(place: Place, width: i32) -> bool {
        let bar = RECT {
            left: place[0],
            top: place[1],
            right: place[0] + width.clamp(1, 400),
            bottom: place[1] + 30,
        };
        !unsafe { MonitorFromRect(&bar, MONITOR_DEFAULTTONULL) }.is_null()
    }

    /// The first top-level dialog window created on this thread (the file
    /// dialog's frame) gets `wrapped` as its window procedure.
    pub unsafe extern "system" fn on_cbt(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code as u32 == HCBT_CREATEWND {
            let hwnd = wparam as HWND;
            let create = lparam as *const CBT_CREATEWNDW;
            let style = unsafe { create.as_ref() }
                .and_then(|c| unsafe { c.lpcs.as_ref() })
                .map_or(WS_CHILD as i32, |cs| cs.style);
            if style as u32 & WS_CHILD == 0 && class_is_dialog(hwnd) {
                STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    if s.dialog.is_none() {
                        let own = unsafe {
                            SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wrapped as Proc as usize as isize)
                        };
                        if own != 0 {
                            s.dialog = Some((hwnd as usize, own));
                        }
                    }
                });
            }
        }
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    unsafe extern "system" fn wrapped(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let own = STATE.with(|s| {
            let Ok(mut s) = s.try_borrow_mut() else {
                return None;
            };
            let (dialog, own) = s.dialog?;
            if dialog != hwnd as usize {
                return Some(own);
            }
            match msg {
                WM_WINDOWPOSCHANGING if !s.shown => {
                    // Until shown, every placement goes to the saved place.
                    if let Some(pos) = unsafe { (lparam as *mut WINDOWPOS).as_mut() } {
                        let showing = pos.flags & SWP_SHOWWINDOW != 0;
                        if let Some(want) = s.want {
                            let width = if pos.cx > 0 {
                                pos.cx
                            } else {
                                rect(hwnd).map_or(400, |r| r.right - r.left)
                            };
                            if (showing || pos.flags & SWP_NOMOVE == 0) && on_screen(want, width) {
                                pos.x = want[0];
                                pos.y = want[1];
                                pos.flags &= !SWP_NOMOVE;
                            }
                        }
                        s.shown |= showing;
                    }
                }
                WM_WINDOWPOSCHANGED if s.shown && unsafe { IsIconic(hwnd) } == 0 => {
                    if let Some(r) = rect(hwnd) {
                        s.last = Some([r.left, r.top]);
                    }
                }
                WM_NCDESTROY => {
                    unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, own) };
                }
                _ => {}
            }
            Some(own)
        });
        // The dialog's own procedure (always found: only the dialog is
        // wrapped, and only while its state is set).
        let Some(own) = own.filter(|p| *p != 0) else {
            return 0;
        };
        let own: Proc = unsafe { std::mem::transmute::<isize, Proc>(own) };
        unsafe { CallWindowProcW(Some(own), hwnd, msg, wparam, lparam) }
    }
}
