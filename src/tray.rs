//! Resident tray mode.
//!
//! The command subcommands are one-shot: they capture, print and exit. This
//! mode exists for the way the tool is used during a working day, where
//! starting a capture should cost one keystroke or one click instead of a
//! terminal command.
//!
//! It owns three pieces of shell state for as long as the process lives:
//!
//! * one icon in the notification area,
//! * one global hotkey,
//! * a hidden window that exists only to receive their messages. It is created
//!   but never shown, because the notification area, the hotkey and the popup
//!   menu only need a window to be addressed by, and nothing about that window
//!   is meant to be visible.
//!
//! Both tray gestures and the hotkey end in the same pipeline the `capture` and
//! `select` subcommands use, so there is one implementation of capturing and
//! one of publishing to the clipboard. The difference is the destination: this
//! mode copies to the clipboard and writes no file, because the output location
//! is not configurable yet and inventing a path per invocation would leave
//! files behind that nothing can find again.
//!
//! The region overlay runs a nested message loop of its own, so tray and hotkey
//! messages keep being delivered while a selection is on screen. A busy flag in
//! the window state is what keeps a second request from re-entering the overlay
//! on top of the first.
//!
//! The icon and the hotkey are acquired independently. A session without a
//! working notification area still gets the hotkey, and in that case the console
//! is left visible: it carries the warning and it is the only way to stop a mode
//! with no icon to click.

use std::cell::RefCell;
use std::mem::size_of;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Console::GetConsoleWindow;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};
use windows::Win32::UI::Shell::{
    ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, HICON, HMENU,
    MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SW_HIDE,
    SW_SHOWNORMAL, SetForegroundWindow, ShowWindow, SM_CXSMICON, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, TranslateMessage, WM_APP, WM_DESTROY, WM_HOTKEY, WM_LBUTTONUP, WM_NULL,
    WM_RBUTTONUP, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};

use crate::{capture, clipboard, hotkey, icon, overlay};

/// Directory the resident mode refers to.
///
/// The same `target/` tree the command subcommands already use, which version
/// control ignores, so a capture that never reaches the clipboard cannot litter
/// a checkout.
const OUTPUT_DIR: &str = "target/test-output";

/// Identifier of the single tray icon, stable for the lifetime of the process.
const TRAY_ICON_ID: u32 = 1;

/// Callback message the shell posts to the hidden window for tray events.
///
/// It starts at `WM_APP`, the first value Windows leaves to applications for
/// messages of their own.
const WM_TRAY: u32 = WM_APP + 1;

/// Identifier of the global hotkey. Only one is registered, and the value only
/// has to be unique within this thread's registrations.
const HOTKEY_ID: i32 = 1;

/// Context menu identifiers.
///
/// The menu reports the chosen identifier as its return value instead of
/// posting `WM_COMMAND`, which keeps the choice handling in one place.
const ID_CAPTURE_SCREEN: usize = 1;
const ID_SELECT_REGION: usize = 2;
const ID_OPEN_OUTPUT: usize = 3;
const ID_QUIT: usize = 4;

/// The shell resources and the window state of the resident mode.
struct State {
    hwnd: HWND,
    /// Icon built from the embedded resource. Unlike a shared system icon, this
    /// one belongs to the process and has to be destroyed on the way out.
    icon: HICON,
    menu: HMENU,
    /// The combination the hotkey was registered with, so the message can be
    /// matched and the failure report can name it.
    hotkey: hotkey::Hotkey,
    /// Set while a capture or the overlay is running, so a request that arrives
    /// during the overlay's nested message loop is ignored instead of stacking
    /// a second overlay on top of it.
    busy: bool,
    icon_added: bool,
    hotkey_registered: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Start the resident mode and return once the message loop has ended.
pub fn run(hotkey: hotkey::Hotkey) -> Result<()> {
    let hmodule = unsafe { GetModuleHandleW(None) }.context("GetModuleHandleW failed")?;
    let hinstance = HINSTANCE(hmodule.0);

    // Both are acquired before the window, so a failure here cannot leave a
    // window behind that nothing is able to close.
    // The small-icon metric is the size the notification area draws at the
    // current scaling, so asking for it here is what keeps the icon sharp
    // instead of letting the shell scale a fixed size.
    let icon_size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    let icon = icon::icon(icon_size)?;
    let menu = create_menu()?;

    let class_name = w!("LassoTray");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance,
        lpszClassName: class_name,
        ..Default::default()
    };
    // A zero return means this process already registered the class, which is
    // expected when the mode is entered twice in one process.
    unsafe {
        RegisterClassW(&wc);
    }

    // Never shown and given no size, because nothing is ever drawn in it;
    // `WS_EX_TOOLWINDOW` keeps it out of the taskbar and the window switcher.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class_name,
            w!("lasso"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance),
            None,
        )
    }
    .context("CreateWindowExW failed")?;

    STATE.with(|slot| {
        *slot.borrow_mut() = Some(State {
            hwnd,
            icon,
            menu,
            hotkey,
            busy: false,
            icon_added: false,
            hotkey_registered: false,
        });
    });

    if let Err(err) = install() {
        // Without the icon there is nothing to click, so the mode gives up and
        // releases whatever it already took.
        dispose();
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(err);
    }

    // Hidden only once the icon is actually in place. Nothing here writes a
    // result to the console, so it is hidden rather than left behind as an empty
    // window — but without the icon the console carries the warning and is the
    // only way left to stop the mode, so it stays.
    if with_state(|state| state.icon_added).unwrap_or(false) {
        hide_console();
    }

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // Reached on every exit path, including a `WM_QUIT` that came from
    // somewhere other than the quit item, so the shell is never left holding an
    // icon or a hotkey for a process that is gone.
    dispose();
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    Ok(())
}

/// Hide the console window the executable was started with.
///
/// A process without its own console, which is what a build with the windows
/// subsystem produces, reports no window here and there is nothing to hide.
fn hide_console() {
    unsafe {
        let console = GetConsoleWindow();
        if !console.is_invalid() {
            let _ = ShowWindow(console, SW_HIDE);
        }
    }
}

/// Add the tray icon and register the hotkey.
///
/// Neither is treated as fatal on its own. The notification area can be missing
/// while the rest of the session works — the shell may have crashed, or the
/// process may run in a session that never started one — and the hotkey does not
/// depend on it. The mode therefore keeps working with whichever of the two it
/// got, and only refuses to start when it got neither.
fn install() -> Result<()> {
    let added = with_state(|state| unsafe {
        Shell_NotifyIconW(NIM_ADD, &notify_data(state.hwnd, state.icon)).as_bool()
    })
    .unwrap_or(false);
    if added {
        with_state(|state| state.icon_added = true);
    } else {
        let code = unsafe { GetLastError() };
        eprintln!(
            "[tray] the notification area refused the icon (Shell_NotifyIconW error {}); \
             continuing without it",
            code.0
        );
    }

    let hwnd = with_state(|state| state.hwnd);
    let Some(hwnd) = hwnd else {
        bail!("the resident window is missing");
    };

    let hotkey = with_state(|state| state.hotkey).unwrap_or(hotkey::Hotkey::DEFAULT);
    match unsafe {
        RegisterHotKey(
            Some(hwnd),
            HOTKEY_ID,
            HOT_KEY_MODIFIERS(hotkey.modifiers.bits()),
            u32::from(hotkey.key),
        )
    } {
        Ok(()) => {
            with_state(|state| state.hotkey_registered = true);
        }
        Err(err) => eprintln!("[tray] RegisterHotKey({hotkey}) failed: {err}"),
    }

    // Neither route in means there is nothing to interact with.
    let usable = with_state(|state| state.icon_added || state.hotkey_registered).unwrap_or(false);
    if !usable {
        bail!("neither the notification area nor the global hotkey is available");
    }
    Ok(())
}

/// Give back everything the shell holds for this process and drop the state.
///
/// The quit item and the end of the message loop both run this, so each step is
/// guarded by its own flag and a second call is a no-op.
fn dispose() {
    STATE.with(|slot| {
        let taken = slot.borrow_mut().take();
        let Some(mut state) = taken else {
            return;
        };

        if state.icon_added {
            let data = notify_data(state.hwnd, state.icon);
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
            state.icon_added = false;
        }
        if state.hotkey_registered {
            unsafe {
                let _ = UnregisterHotKey(Some(state.hwnd), HOTKEY_ID);
            }
            state.hotkey_registered = false;
        }
        unsafe {
            let _ = DestroyMenu(state.menu);
            // Created by this process rather than borrowed from the system, so
            // it is ours to release.
            let _ = DestroyIcon(state.icon);
        }
    });
}

/// Run `f` against the resident state while the mode owns one.
fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> Option<T> {
    STATE.with(|slot| slot.borrow_mut().as_mut().map(f))
}

/// The notification data describing the single tray icon.
///
/// `cbSize` is the size of the whole structure, which is what current shells
/// expect; the smaller legacy sizes exist for older systems only.
fn notify_data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    set_tip(&mut data, "lasso");
    data
}

/// Copy the tooltip into the inline buffer the structure carries.
///
/// The field is a fixed array rather than a pointer, so the text is truncated
/// to fit and always left null-terminated.
fn set_tip(data: &mut NOTIFYICONDATAW, tip: &str) {
    let units: Vec<u16> = tip.encode_utf16().take(data.szTip.len() - 1).collect();
    data.szTip[..units.len()].copy_from_slice(&units);
}

/// Build the context menu once; it is reused by every left click.
fn create_menu() -> Result<HMENU> {
    unsafe {
        let menu = CreatePopupMenu().context("CreatePopupMenu failed")?;

        let built = (|| -> Result<()> {
            AppendMenuW(menu, MF_STRING, ID_CAPTURE_SCREEN, w!("Capture screen"))
                .context("AppendMenuW(Capture screen) failed")?;
            AppendMenuW(menu, MF_STRING, ID_SELECT_REGION, w!("Select region"))
                .context("AppendMenuW(Select region) failed")?;
            AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())
                .context("AppendMenuW(separator) failed")?;
            AppendMenuW(menu, MF_STRING, ID_OPEN_OUTPUT, w!("Open output folder"))
                .context("AppendMenuW(Open output folder) failed")?;
            AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Quit"))
                .context("AppendMenuW(Quit) failed")?;
            Ok(())
        })();

        if let Err(err) = built {
            let _ = DestroyMenu(menu);
            return Err(err);
        }
        Ok(menu)
    }
}

/// Capture the whole virtual desktop and put it on the clipboard.
fn capture_to_clipboard() -> Result<()> {
    let frame = capture::capture_virtual_desktop(None)?;
    clipboard::copy_bgra(&frame.bgra, frame.width, frame.height)
}

/// Capture, let the user mark a region, and put that region on the clipboard.
///
/// Cancelling the overlay is not a failure: nothing happens and the mode stays
/// ready for the next request.
fn select_to_clipboard() -> Result<()> {
    let frame = capture::capture_virtual_desktop(None)?;
    let Some(selection) = overlay::select_region(&frame)? else {
        return Ok(());
    };

    let region = frame
        .to_bgra_sub(selection.x, selection.y, selection.width, selection.height)
        .context("the selected region falls outside the capture")?;
    clipboard::copy_bgra(&region, selection.width, selection.height)
}

/// Run a capture action unless one is already running.
///
/// The overlay pumps its own message loop, so this can be entered again from
/// inside the overlay's loop; the busy flag is what turns that into a no-op.
fn start(action: fn() -> Result<()>) {
    let idle = with_state(|state| {
        let idle = !state.busy;
        state.busy = true;
        idle
    })
    .unwrap_or(false);
    if !idle {
        return;
    }

    // The console is hidden in this mode, so a failure has nowhere to be read
    // from; it is still reported rather than swallowed.
    if let Err(err) = action() {
        eprintln!("[tray] {err:#}");
    }

    with_state(|state| state.busy = false);
}

/// Show the context menu at the pointer.
///
/// The shell requires the owning window to be in the foreground around the
/// call, otherwise the menu stays on screen after a click elsewhere; the
/// trailing null message is what ends that handover cleanly.
fn show_menu() {
    let Some((hwnd, menu)) = with_state(|state| (state.hwnd, state.menu)) else {
        return;
    };

    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        return;
    }

    let chosen = unsafe {
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD,
            point.x,
            point.y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        chosen.0
    };

    // A dismissed menu reports zero, which matches no item.
    match chosen as usize {
        ID_CAPTURE_SCREEN => start(capture_to_clipboard),
        ID_SELECT_REGION => start(select_to_clipboard),
        ID_OPEN_OUTPUT => open_output_folder(hwnd),
        ID_QUIT => quit(),
        _ => {}
    }
}

/// Open the capture destination in the default file manager.
///
/// The path is made absolute before it is handed over, because the handler
/// resolves a relative one against its own working directory rather than this
/// process's.
fn open_output_folder(hwnd: HWND) {
    if let Err(err) = std::fs::create_dir_all(OUTPUT_DIR) {
        eprintln!("[tray] could not create {OUTPUT_DIR}: {err}");
        return;
    }

    let folder = std::path::absolute(OUTPUT_DIR).unwrap_or_else(|_| PathBuf::from(OUTPUT_DIR));
    let path = wide(&folder.to_string_lossy());

    unsafe {
        let _ = ShellExecuteW(
            Some(hwnd),
            w!("open"),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Leave the mode.
///
/// The icon and the hotkey go first, so the shell never shows an icon that
/// belongs to a message loop which is about to stop.
fn quit() {
    dispose();
    unsafe {
        PostQuitMessage(0);
    }
}

/// Encode a path as the null-terminated wide string the Win32 calls expect.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // The icon was added without asking for the newer notification layout,
        // so the mouse message arrives on its own in `lParam`.
        WM_TRAY => {
            match lparam.0 as u32 {
                WM_LBUTTONUP => show_menu(),
                WM_RBUTTONUP => start(select_to_clipboard),
                _ => {}
            }
            LRESULT(0)
        }

        WM_HOTKEY => {
            if wparam.0 as i32 == HOTKEY_ID {
                start(select_to_clipboard);
            }
            LRESULT(0)
        }

        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }

        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
