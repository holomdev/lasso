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
//! `select` subcommands use, so there is one implementation of capturing. Where
//! the result goes is decided by the settings file: the clipboard, a file in the
//! chosen folder, or both.
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
use windows::Win32::System::Console::{GetConsoleProcessList, GetConsoleWindow};
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
    MB_ICONINFORMATION, MB_OK, MessageBoxW, MF_SEPARATOR, MF_STRING, MSG, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SW_HIDE,
    SW_SHOWNORMAL, SetForegroundWindow, SetTimer, KillTimer, ShowWindow, SM_CXSMICON,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_DESTROY,
    WM_HOTKEY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW,
    WS_POPUP,
};

use crate::settings::{Settings, TrayClick};
use crate::{capture, clipboard, hotkey, icon, overlay, settings, settings_window};

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

/// Timer used while the notification area has not accepted the icon yet.
const RETRY_TIMER_ID: usize = 1;

/// How long to wait between attempts at the icon, in milliseconds.
const RETRY_INTERVAL_MS: u32 = 5000;

/// How many times the icon is attempted at startup before giving up.
///
/// The shell can be slow to start and the mode may be launched before it. After
/// these run out the icon is only attempted again when the shell announces that
/// it has restarted, which is the case that can happen at any time.
const ICON_RETRIES: u32 = 6;

/// Context menu identifiers.
///
/// The menu reports the chosen identifier as its return value instead of
/// posting `WM_COMMAND`, which keeps the choice handling in one place.
const ID_CAPTURE_SCREEN: usize = 1;
const ID_SELECT_REGION: usize = 2;
const ID_OPEN_OUTPUT: usize = 3;
const ID_SETTINGS: usize = 4;
const ID_ABOUT: usize = 5;
const ID_QUIT: usize = 6;

/// The shell resources and the window state of the resident mode.
struct State {
    hwnd: HWND,
    /// Icon built from the embedded resource. Unlike a shared system icon, this
    /// one belongs to the process and has to be destroyed on the way out.
    icon: HICON,
    menu: HMENU,
    /// The settings this run is using, so a capture can consult them without
    /// reading the file again.
    settings: Settings,
    /// The id of the message the shell sends to every top-level window when it
    /// restarts. An icon belongs to the shell that accepted it, so the one this
    /// process added disappears with it and has to be added again.
    taskbar_created: u32,
    /// Attempts left at putting the icon in the notification area.
    retries_left: u32,
    /// Set once the console has been hidden, so it only ever happens once.
    console_hidden: bool,
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
pub fn run(hotkey_override: Option<hotkey::Hotkey>) -> Result<()> {
    // The settings are read first, because the hotkey the mode registers comes
    // from them; a combination on the command line replaces it for this run only
    // and is not written back.
    let mut settings = settings::Settings::load();
    if let Some(hotkey) = hotkey_override {
        settings.hotkey = hotkey;
    }

    // Looked up rather than assumed: the message only has a number once the shell
    // has registered it, and that number is what identifies it from then on.
    let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };

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
            settings,
            taskbar_created,
            retries_left: ICON_RETRIES,
            console_hidden: false,
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

    // Hidden only once the icon is actually in place, which `retry_icon` reports
    // by calling this. Nothing here writes a result to the console, so it is
    // hidden rather than left behind as an empty window.
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

/// Try to put the icon in the notification area.
///
/// A refusal is not final: the shell can be slow to start, and there are
/// environments where the call is refused outright. The caller decides what to do
/// about it.
fn add_icon() -> bool {
    let added = with_state(|state| unsafe {
        Shell_NotifyIconW(NIM_ADD, &notify_data(state.hwnd, state.icon)).as_bool()
    })
    .unwrap_or(false);

    if added {
        let hwnd = with_state(|state| {
            state.icon_added = true;
            state.hwnd
        });
        if let Some(hwnd) = hwnd {
            unsafe {
                let _ = KillTimer(Some(hwnd), RETRY_TIMER_ID);
            }
        }
    }
    added
}

/// Install the icon, and arrange another attempt when it is refused.
fn retry_icon() {
    if with_state(|state| state.icon_added).unwrap_or(false) {
        return;
    }

    if add_icon() {
        eprintln!("[tray] the icon is in the notification area");
        // With the icon up, the console is no longer the only way to stop this.
        hide_console_once();
        return;
    }

    let code = unsafe { GetLastError() };
    let scheduled = with_state(|state| {
        state.retries_left = state.retries_left.saturating_sub(1);
        (state.retries_left, state.hwnd)
    });

    match scheduled {
        Some((left, hwnd)) if left > 0 => {
            eprintln!(
                "[tray] the notification area refused the icon (error {}); \
                 trying again in {} s",
                code.0,
                RETRY_INTERVAL_MS / 1000
            );
            unsafe {
                SetTimer(Some(hwnd), RETRY_TIMER_ID, RETRY_INTERVAL_MS, None);
            }
        }
        _ => {
            eprintln!(
                "[tray] the notification area refused the icon (error {}); continuing without \
                 it, and trying again if the shell restarts",
                code.0
            );
        }
    }
}

/// The shell has restarted, which takes every icon with it.
fn on_taskbar_created() {
    with_state(|state| {
        state.icon_added = false;
        // A fresh shell is a fresh chance, however many attempts were used up.
        state.retries_left = ICON_RETRIES;
    });
    retry_icon();
}

/// Hide the console, but only when it belongs to this process.
///
/// A console shared with a shell belongs to that shell, so hiding it would make
/// the user's own terminal window disappear — and starting the resident mode from
/// a terminal is an ordinary thing to do.
fn hide_console_once() {
    if with_state(|state| state.console_hidden).unwrap_or(true) {
        return;
    }
    if !owns_console() {
        return;
    }
    hide_console();
    with_state(|state| state.console_hidden = true);
}

/// Whether the console this process uses was created for it alone.
fn owns_console() -> bool {
    let mut processes = [0u32; 2];
    // The return is how many processes are attached. A number larger than the
    // buffer means more than the two asked about, so it is shared.
    let attached = unsafe { GetConsoleProcessList(&mut processes) };
    attached <= 1
}

/// Add the tray icon and register the hotkey.
///
/// Neither is treated as fatal on its own. The notification area can be missing
/// while the rest of the session works — the shell may have crashed, or the
/// process may run in a session that never started one — and the hotkey does not
/// depend on it. The mode therefore keeps working with whichever of the two it
/// got, and only refuses to start when it got neither.
fn install() -> Result<()> {
    // One path for every attempt: a shell that is not ready yet, a shell that has
    // just restarted, and a notification area that is not there at all.
    retry_icon();

    let hotkey = with_state(|state| state.settings.hotkey).unwrap_or(hotkey::Hotkey::DEFAULT);
    match register_hotkey(hotkey) {
        Ok(()) => {
            with_state(|state| state.hotkey_registered = true);
        }
        // Reported rather than fatal: a combination another program already owns
        // leaves the mode running with the icon alone.
        Err(err) => eprintln!("[tray] {err:#}"),
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
///
/// Shared state behind a window procedure has to survive re-entry: several Win32
/// calls made here send messages that come straight back. A plain `borrow_mut`
/// would panic, and a panic inside a window procedure cannot unwind, which turns
/// a recoverable mistake into an aborted process.
fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> Option<T> {
    STATE.with(|slot| slot.try_borrow_mut().ok()?.as_mut().map(f))
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
            AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())
                .context("AppendMenuW(separator) failed")?;
            AppendMenuW(menu, MF_STRING, ID_SETTINGS, w!("Settings..."))
                .context("AppendMenuW(Settings) failed")?;
            AppendMenuW(menu, MF_STRING, ID_ABOUT, w!("About lasso"))
                .context("AppendMenuW(About) failed")?;
            AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())
                .context("AppendMenuW(separator) failed")?;
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

/// Capture the whole virtual desktop and publish it.
fn capture_screen() -> Result<()> {
    let frame = capture::capture_virtual_desktop(None)?;
    publish(&frame, None)
}

/// Capture, let the user mark a region, and publish that region.
///
/// Cancelling the overlay is not a failure: nothing happens and the mode stays
/// ready for the next request.
fn select_region() -> Result<()> {
    let frame = capture::capture_virtual_desktop(None)?;
    let Some(selection) = overlay::select_region(&frame)? else {
        return Ok(());
    };
    publish(&frame, Some(selection))
}

/// Send a capture wherever the settings ask for.
///
/// The two destinations are independent, so a capture can go to the clipboard,
/// to a file, or to both.
fn publish(frame: &capture::Frame, region: Option<overlay::Selection>) -> Result<()> {
    let settings = with_state(|state| state.settings.clone()).unwrap_or_default();
    if !settings.clipboard && !settings.save_file {
        return Ok(());
    }

    let (x, y, width, height) = match region {
        Some(selection) => (
            selection.x,
            selection.y,
            selection.width,
            selection.height,
        ),
        None => (0, 0, frame.width, frame.height),
    };

    if settings.clipboard {
        let pixels = frame
            .to_bgra_sub(x, y, width, height)
            .context("the region falls outside the capture")?;
        clipboard::copy_bgra(&pixels, width, height)?;
    }

    if settings.save_file {
        let image = match region {
            Some(_) => frame.to_rgba_sub(x, y, width, height),
            None => Some(frame.to_rgba_image()),
        }
        .context("the region falls outside the capture")?;

        let path = saved_path(&settings.output_dir);
        if let Some(parent) = std::path::Path::new(&path).parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        image
            .save(&path)
            .with_context(|| format!("could not write {path}"))?;
    }

    Ok(())
}

/// A fresh file name for a saved capture.
///
/// The timestamp is what keeps two captures from overwriting each other without
/// asking the user anything.
fn saved_path(directory: &str) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    std::path::Path::new(directory)
        .join(format!("lasso-{stamp}.png"))
        .to_string_lossy()
        .into_owned()
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
        ID_CAPTURE_SCREEN => start(capture_screen),
        ID_SELECT_REGION => start(select_region),
        ID_OPEN_OUTPUT => open_output_folder(hwnd),
        ID_SETTINGS => open_settings(),
        ID_ABOUT => show_about(hwnd),
        ID_QUIT => quit(),
        _ => {}
    }
}

/// Register the mode's single global hotkey.
fn register_hotkey(hotkey: hotkey::Hotkey) -> Result<()> {
    let hwnd = with_state(|state| state.hwnd).context("the resident window is missing")?;
    unsafe {
        RegisterHotKey(
            Some(hwnd),
            HOTKEY_ID,
            HOT_KEY_MODIFIERS(hotkey.modifiers.bits()),
            u32::from(hotkey.key),
        )
    }
    .with_context(|| format!("RegisterHotKey({hotkey}) was refused"))
}

/// Open the settings window and apply whatever comes back.
///
/// Guarded the same way a capture is: while the window is up, another request
/// must not start on top of it.
fn open_settings() {
    let idle = with_state(|state| {
        let idle = !state.busy;
        state.busy = true;
        idle
    })
    .unwrap_or(false);
    if !idle {
        return;
    }

    if let Err(err) = apply_settings() {
        eprintln!("[tray] settings: {err:#}");
    }

    with_state(|state| state.busy = false);
}

/// Show the window, then save and re-register whatever changed.
fn apply_settings() -> Result<()> {
    let Some((owner, current)) = with_state(|state| (state.hwnd, state.settings.clone())) else {
        return Ok(());
    };

    let Some(updated) = settings_window::open(owner, &current)? else {
        // Cancelled: nothing to do and nothing to report.
        return Ok(());
    };

    // The new combination is registered before the old one is dropped, so a
    // refused shortcut cannot leave the mode with none at all.
    if updated.hotkey != current.hotkey {
        register_hotkey(updated.hotkey).with_context(|| {
            format!(
                "{} is still the shortcut, because the new one could not be taken",
                current.hotkey
            )
        })?;
        if let Some(hwnd) = with_state(|state| state.hwnd) {
            unsafe {
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
            }
        }
    }

    if updated != current {
        updated.save().context("could not save the settings")?;
    }

    with_state(|state| state.settings = updated);
    with_state(|state| state.hotkey_registered = true);
    Ok(())
}

/// Describe the program.
///
/// Reachable both from the tray menu and from `lasso about`, which is why it is
/// crate-visible rather than private to this module.
pub(crate) fn show_about(owner: HWND) {
    let message = format!(
        "lasso {}\n\nA screenshot tool for Windows.\n\nLicense: {}\n{}",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_LICENSE"),
        env!("CARGO_PKG_REPOSITORY")
    );
    let text = wide(&message);
    let caption = wide("About lasso");
    unsafe {
        MessageBoxW(
            Some(owner),
            PCWSTR::from_raw(text.as_ptr()),
            PCWSTR::from_raw(caption.as_ptr()),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

/// Look at a directory in the default file manager.
///
/// The path is made absolute before it is handed over, because the handler
/// resolves a relative one against its own working directory rather than this
/// process's.
fn open_output_folder(hwnd: HWND) {
    let directory = with_state(|state| state.settings.output_dir.clone())
        .unwrap_or_else(|| settings::Settings::default().output_dir);

    if let Err(err) = std::fs::create_dir_all(&directory) {
        eprintln!("[tray] could not create {directory}: {err}");
        return;
    }

    let folder = std::path::absolute(&directory).unwrap_or_else(|_| PathBuf::from(&directory));
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
    // The number of this message is only known at run time, so it cannot be a
    // match arm.
    if msg != 0 && with_state(|state| state.taskbar_created) == Some(msg) {
        on_taskbar_created();
        return LRESULT(0);
    }

    match msg {
        WM_TIMER if wparam.0 == RETRY_TIMER_ID => {
            retry_icon();
            LRESULT(0)
        }
        // The icon was added without asking for the newer notification layout,
        // so the mouse message arrives on its own in `lParam`.
        WM_TRAY => {
            // Which button opens the menu is a setting; the other one starts a
            // selection straight away, which is the whole point of the icon.
            let capture_on_left =
                with_state(|state| state.settings.tray_left_click).unwrap_or(TrayClick::Menu)
                    == TrayClick::Capture;
            match lparam.0 as u32 {
                WM_LBUTTONUP if capture_on_left => start(select_region),
                WM_RBUTTONUP if !capture_on_left => start(select_region),
                WM_LBUTTONUP | WM_RBUTTONUP => show_menu(),
                _ => {}
            }
            LRESULT(0)
        }

        WM_HOTKEY => {
            if wparam.0 as i32 == HOTKEY_ID {
                start(select_region);
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
