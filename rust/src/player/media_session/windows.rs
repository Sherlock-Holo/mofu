//! Hidden HWND required by Windows System Media Transport Controls.

use std::ffi::c_void;
use std::io;
use std::mem;
use std::ptr::NonNull;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GA_ROOT, GetAncestor,
    IsDialogMessageW, MSG, PM_REMOVE, PeekMessageW, RegisterClassExW, TranslateMessage,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_QUIT, WNDCLASSEXW,
};
use windows::core::w;

/// Message-only-style window kept alive for SMTC registration.
///
/// Stores a non-null raw pointer so the handle can move across threads with the media-session
/// backend. Window creation and message pumping remain the caller's responsibility.
pub struct DummyWindow {
    handle: NonNull<c_void>,
}

// `NonNull<c_void>` is not Send; the pointer is only used as SMTC registration context.
unsafe impl Send for DummyWindow {}

impl DummyWindow {
    pub fn new() -> io::Result<Self> {
        let class_name = w!("mofu_media_session");
        let handle = unsafe {
            let instance = GetModuleHandleW(None)
                .map_err(|err| io::Error::other(format!("GetModuleHandleW failed: {err}")))?;

            let wnd_class = WNDCLASSEXW {
                cbSize: mem::size_of::<WNDCLASSEXW>() as u32,
                hInstance: instance.into(),
                lpszClassName: class_name,
                lpfnWndProc: Some(Self::wnd_proc),
                ..Default::default()
            };

            // Re-creating the player may register the same class again; ignore that failure.
            let _ = RegisterClassExW(&wnd_class);

            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                w!(""),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|err| io::Error::other(format!("CreateWindowExW failed: {err}")))?
        };
        let handle = NonNull::new(handle.0).ok_or_else(io::Error::last_os_error)?;
        Ok(Self { handle })
    }

    pub fn hwnd(&self) -> *mut c_void {
        self.handle.as_ptr()
    }

    /// Drain the Windows message queue so SMTC events are delivered.
    pub fn pump_event_queue() {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break;
                }
                if !IsDialogMessageW(GetAncestor(msg.hwnd, GA_ROOT), &msg).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    }

    extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}

impl Drop for DummyWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(HWND(self.handle.as_ptr()));
        }
    }
}
