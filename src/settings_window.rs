//! The settings window.
//!
//! Opened from the resident mode's menu. It is modal in practice: the tray window
//! is disabled while it is up and a nested message loop runs until Save or
//! Cancel, so the caller receives the new settings or nothing at all.
//!
//! Two decisions keep this small.
//!
//! The controls are created in plain code after the window exists, not in
//! `WM_CREATE`. `WM_CREATE` runs inside `CreateWindowExW`, before `open` has had
//! any chance to store the owner and the settings the controls have to be filled
//! from, so building them there would mean passing everything through a window
//! creation parameter. Doing it afterwards is the same work in a straight line.
//!
//! The hotkey field is a read-only edit whose window procedure is replaced. A
//! read-only edit still takes focus and still receives key messages, which makes
//! it the smallest thing that can capture a combination; a control painted from
//! scratch would be a lot of code for a field that shows one line.
//!
//! Nothing here writes anything. It hands the edited settings back and the
//! resident mode decides what to do with them, which keeps saving the file and
//! re-registering the hotkey in one place.

use std::cell::RefCell;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{DEFAULT_GUI_FONT, GetStockObject};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, SetFocus, VK_CONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RCONTROL,
    VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_MENU,
};
use windows::Win32::UI::Shell::{
    BIF_EDITBOX, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, ILFree, SHBrowseForFolderW,
    SHGetPathFromIDListW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_WNDPROC,
    GetMessageW, GetSystemMetrics, GetWindowRect, GetWindowTextW, HMENU, HWND_TOP, MSG,
    RegisterClassW, SM_CXSCREEN, SM_CYSCREEN, SWP_NOZORDER, SW_SHOW, SendMessageW,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow,
    TranslateMessage, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_KEYDOWN, WM_NCDESTROY, WM_SETFONT,
    WM_SYSKEYDOWN, WNDCLASSW, WNDPROC, WS_CAPTION, WS_CHILD, WS_EX_DLGMODALFRAME, WS_OVERLAPPED,
    WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WINDOW_STYLE,
};
use windows::core::{PCWSTR, w};

use crate::hotkey::{Hotkey, Modifiers};
use crate::settings::{Settings, TrayClick};

/// Control styles and messages whose values come from the Win32 headers.
///
/// They are spelled out because the crate models several of them as newtypes over
/// the same integers while `CreateWindowExW` wants plain style bits, and mixing
/// the two is noise for six constants. Every value is the one the headers define.
const BS_AUTOCHECKBOX: u32 = 0x0000_0003;
const BS_PUSHBUTTON: u32 = 0x0000_0000;
const ES_AUTOHSCROLL: u32 = 0x0000_0080;
const ES_READONLY: u32 = 0x0000_0800;
const SS_LEFT: u32 = 0x0000_0000;
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const BST_CHECKED: isize = 1;
const BST_UNCHECKED: isize = 0;

/// Layout, in client coordinates. A fixed size is honest here: the window holds a
/// known set of controls and nothing in it reflows.
const PAD: i32 = 16;
const LABEL_WIDTH: i32 = 104;
const FIELD_HEIGHT: i32 = 24;
const CLIENT_WIDTH: i32 = 460;
const CLIENT_HEIGHT: i32 = 336;
const BUTTON_WIDTH: i32 = 92;

/// Identifiers, so a message can say which control it came from.
const ID_HOTKEY: i32 = 100;
const ID_HOTKEY_HINT: i32 = 101;
const ID_CLIPBOARD: i32 = 102;
const ID_SAVE_FILE: i32 = 103;
const ID_OUTPUT_DIR: i32 = 104;
const ID_BROWSE: i32 = 105;
const ID_OUTPUT_HINT: i32 = 106;
const ID_TRAY_CLICK: i32 = 107;
const ID_TRAY_CLICK_HINT: i32 = 108;
const ID_SAVE: i32 = 109;
const ID_CANCEL: i32 = 110;

/// What the window says.
const HOTKEY_HINT: &str = "Click the field and press a combination. A modifier is required.";
const HOTKEY_NEEDS_MODIFIER: &str =
    "Refused: without a modifier that key would be taken from every other program.";
const OUTPUT_HINT: &str =
    "Only used when saving to a file. Names carry the time the capture was taken.";
const OUTPUT_NEEDS_FOLDER: &str = "Refused: choose an output folder before saving.";
const TRAY_CLICK_HINT: &str = "The other mouse button always starts a selection.";

struct State {
    window: HWND,
    owner: HWND,
    hotkey_field: HWND,
    hotkey_hint: HWND,
    clipboard: HWND,
    save_file: HWND,
    output_dir: HWND,
    output_hint: HWND,
    tray_click: HWND,
    tray_click_hint: HWND,
    /// The combination the field holds. Never `None` in practice: the window
    /// refuses to save without one, because the settings model requires it.
    hotkey: Option<Hotkey>,
    tray_left_click: TrayClick,
    /// The settings being edited. Keeping the value that was loaded is what makes
    /// entries this window does not show survive the round trip.
    current: Settings,
    /// The edit's original window procedure, so every message this module does not
    /// handle still reaches it.
    previous_proc: isize,
    done: bool,
    result: Option<Settings>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Run `f` against the window state, or nothing at all when it is already in use.
///
/// The state is thread-local and several of the Win32 calls made here send
/// messages that come straight back into this module: writing to the hotkey field
/// runs its window procedure, which reads the same state. A plain `borrow_mut`
/// would panic in that case, and a panic inside a window procedure cannot unwind,
/// so it would abort the process instead of failing one call. This is the guard
/// that keeps re-entry from becoming fatal.
fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> Option<T> {
    STATE.with(|slot| slot.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Show the window and block until the user is finished with it.
///
/// Returns the edited settings, or `None` when the window was cancelled or closed
/// without saving.
pub fn open(owner: HWND, current: &Settings) -> Result<Option<Settings>> {
    let instance =
        HINSTANCE(unsafe { GetModuleHandleW(None) }.context("GetModuleHandleW failed")?.0);

    let class = w!("LassoSettings");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class,
        ..Default::default()
    };
    // A zero return also means this process already registered the class, which is
    // what a second visit to the settings looks like.
    unsafe {
        let _ = RegisterClassW(&wc);
    }

    let style = WINDOW_STYLE(WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0);
    let ex_style = WS_EX_DLGMODALFRAME;

    // The size given to CreateWindowExW covers the whole window, caption and
    // frame included, while the layout is written in client coordinates. Turning
    // one into the other here is what keeps the bottom row of buttons inside the
    // window instead of below its edge.
    let mut frame = RECT {
        left: 0,
        top: 0,
        right: CLIENT_WIDTH,
        bottom: CLIENT_HEIGHT,
    };
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::AdjustWindowRectEx(
            &mut frame, style, false, ex_style,
        );
    }

    let window = unsafe {
        CreateWindowExW(
            ex_style,
            class,
            w!("lasso settings"),
            style,
            0,
            0,
            frame.right - frame.left,
            frame.bottom - frame.top,
            Some(owner),
            None,
            Some(instance),
            None,
        )
    }
    .context("could not create the settings window")?;

    // The state is installed before anything can go wrong, so a failure below can
    // always tear the window down through the same path.
    STATE.with(|slot| {
        *slot.borrow_mut() = Some(State {
            window,
            owner,
            hotkey_field: HWND::default(),
            hotkey_hint: HWND::default(),
            clipboard: HWND::default(),
            save_file: HWND::default(),
            output_dir: HWND::default(),
            output_hint: HWND::default(),
            tray_click: HWND::default(),
            tray_click_hint: HWND::default(),
            hotkey: Some(current.hotkey),
            tray_left_click: current.tray_left_click,
            current: current.clone(),
            previous_proc: 0,
            done: false,
            result: None,
        });
    });

    if let Err(err) = build_controls(window, instance) {
        unsafe { let _ = DestroyWindow(window); }
        STATE.with(|slot| *slot.borrow_mut() = None);
        return Err(err);
    }
    load_values();

    center(window, owner);
    unsafe {
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(window, SW_SHOW);
        let _ = SetForegroundWindow(window);
        if let Some(field) = with_state(|state| state.hotkey_field) {
            let _ = SetFocus(Some(field));
        }
    }

    // A nested loop rather than a modal dialog box: the resident mode already runs
    // an ordinary message pump and this runs inside it. `WM_QUIT` is deliberately
    // never posted here, because that would end the resident loop too; the loop
    // ends on the flag the window sets when it is destroyed.
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        if with_state(|state| state.done).unwrap_or(true) {
            break;
        }
    }

    let result = with_state(|state| state.result.take()).flatten();
    let owner = with_state(|state| state.owner).unwrap_or_default();
    STATE.with(|slot| *slot.borrow_mut() = None);

    if !owner.is_invalid() {
        unsafe {
            let _ = EnableWindow(owner, true);
        }
    }
    Ok(result)
}

/// Create every control.
fn build_controls(window: HWND, instance: HINSTANCE) -> Result<()> {
    let field_width = CLIENT_WIDTH - PAD * 2 - LABEL_WIDTH;

    let hotkey_label = child(window, w!("STATIC"), "Hotkey", SS_LEFT, -1, (PAD, PAD + 4, LABEL_WIDTH, 18), instance)?;
    let hotkey_field = child(
        window,
        w!("EDIT"),
        "",
        ES_READONLY | ES_AUTOHSCROLL,
        ID_HOTKEY,
        (PAD + LABEL_WIDTH, PAD, field_width, FIELD_HEIGHT),
        instance,
    )?;
    let hotkey_hint = child(
        window,
        w!("STATIC"),
        HOTKEY_HINT,
        SS_LEFT,
        ID_HOTKEY_HINT,
        (PAD + LABEL_WIDTH, PAD + FIELD_HEIGHT + 2, field_width, 30),
        instance,
    )?;

    let clipboard = child(
        window,
        w!("BUTTON"),
        "Copy each capture to the clipboard",
        BS_AUTOCHECKBOX | WS_TABSTOP.0,
        ID_CLIPBOARD,
        (PAD, PAD + 62, CLIENT_WIDTH - PAD * 2, 20),
        instance,
    )?;
    let save_file = child(
        window,
        w!("BUTTON"),
        "Also save each capture to a file",
        BS_AUTOCHECKBOX | WS_TABSTOP.0,
        ID_SAVE_FILE,
        (PAD, PAD + 86, CLIENT_WIDTH - PAD * 2, 20),
        instance,
    )?;

    let output_label = child(window, w!("STATIC"), "Output folder", SS_LEFT, -1, (PAD, PAD + 118, LABEL_WIDTH, 18), instance)?;
    let browse_width = 84;
    let output_dir = child(
        window,
        w!("EDIT"),
        "",
        ES_AUTOHSCROLL | WS_TABSTOP.0,
        ID_OUTPUT_DIR,
        (
            PAD + LABEL_WIDTH,
            PAD + 114,
            CLIENT_WIDTH - PAD * 2 - LABEL_WIDTH - browse_width - 8,
            FIELD_HEIGHT,
        ),
        instance,
    )?;
    let browse = child(
        window,
        w!("BUTTON"),
        "Browse...",
        BS_PUSHBUTTON | WS_TABSTOP.0,
        ID_BROWSE,
        (
            CLIENT_WIDTH - PAD - browse_width,
            PAD + 114,
            browse_width,
            FIELD_HEIGHT,
        ),
        instance,
    )?;
    let output_hint = child(
        window,
        w!("STATIC"),
        OUTPUT_HINT,
        SS_LEFT,
        ID_OUTPUT_HINT,
        (PAD + LABEL_WIDTH, PAD + 138, field_width, 30),
        instance,
    )?;

    let tray_click = child(
        window,
        w!("BUTTON"),
        "",
        BS_PUSHBUTTON | WS_TABSTOP.0,
        ID_TRAY_CLICK,
        (PAD, PAD + 172, CLIENT_WIDTH - PAD * 2, 26),
        instance,
    )?;
    let tray_click_hint = child(
        window,
        w!("STATIC"),
        TRAY_CLICK_HINT,
        SS_LEFT,
        ID_TRAY_CLICK_HINT,
        (PAD, PAD + 202, CLIENT_WIDTH - PAD * 2, 18),
        instance,
    )?;

    let save = child(
        window,
        w!("BUTTON"),
        "Save",
        BS_PUSHBUTTON | WS_TABSTOP.0,
        ID_SAVE,
        (CLIENT_WIDTH - PAD - BUTTON_WIDTH * 2 - 8, CLIENT_HEIGHT - PAD - 26, BUTTON_WIDTH, 26),
        instance,
    )?;
    let cancel = child(
        window,
        w!("BUTTON"),
        "Cancel",
        BS_PUSHBUTTON | WS_TABSTOP.0,
        ID_CANCEL,
        (CLIENT_WIDTH - PAD - BUTTON_WIDTH, CLIENT_HEIGHT - PAD - 26, BUTTON_WIDTH, 26),
        instance,
    )?;
    let _ = (hotkey_label, output_label, browse, save, cancel);

    // Capture keys on the field itself. A read-only edit still receives them.
    // The function is cast through a pointer rather than straight to an integer,
    // which is what the language requires.
    let previous = unsafe {
        SetWindowLongPtrW(
            hotkey_field,
            GWLP_WNDPROC,
            hotkey_proc as *const () as isize,
        )
    };

    with_state(|state| {
        state.hotkey_field = hotkey_field;
        state.hotkey_hint = hotkey_hint;
        state.clipboard = clipboard;
        state.save_file = save_file;
        state.output_dir = output_dir;
        state.output_hint = output_hint;
        state.tray_click = tray_click;
        state.tray_click_hint = tray_click_hint;
        state.previous_proc = previous;
    });
    Ok(())
}

/// Create one child control.
fn child(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    style: u32,
    id: i32,
    rect: (i32, i32, i32, i32),
    instance: HINSTANCE,
) -> Result<HWND> {
    let text = wide(text);
    let control = unsafe {
        CreateWindowExW(
            Default::default(),
            class,
            PCWSTR::from_raw(text.as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | style),
            rect.0,
            rect.1,
            rect.2,
            rect.3,
            Some(parent),
            // The identifier travels as the menu handle, which is how a child
            // control gets one.
            Some(HMENU(id as usize as *mut _)),
            Some(instance),
            None,
        )
    }
    .with_context(|| format!("could not create control {id}"))?;

    // One font for everything, so the window does not mix system and default.
    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    unsafe {
        SendMessageW(
            control,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
    }
    Ok(control)
}

/// Put the loaded settings into the controls.
///
/// Every handle is read out first and written to afterwards. Writing to the
/// hotkey field sends it a message, its window procedure runs, and that procedure
/// reads the same state; doing the writes while the borrow is still held would
/// re-enter it.
fn load_values() {
    let Some(values) = with_state(|state| {
        (
            state.hotkey,
            state.current.output_dir.clone(),
            state.current.clipboard,
            state.current.save_file,
            state.tray_left_click,
            state.hotkey_field,
            state.hotkey_hint,
            state.output_dir,
            state.output_hint,
            state.tray_click,
            state.clipboard,
            state.save_file,
        )
    }) else {
        return;
    };
    let (
        hotkey,
        output_dir,
        clipboard_on,
        save_on,
        tray_left_click,
        hotkey_field,
        hotkey_hint,
        output_control,
        output_hint_control,
        tray_control,
        clipboard_control,
        save_control,
    ) = values;

    let _ = set_text(hotkey_field, &hotkey_text(hotkey));
    let _ = set_text(hotkey_hint, HOTKEY_HINT);
    let _ = set_text(output_control, &output_dir);
    let _ = set_text(output_hint_control, OUTPUT_HINT);
    let _ = set_text(tray_control, &tray_click_text(tray_left_click));
    set_checked(clipboard_control, clipboard_on);
    set_checked(save_control, save_on);
}

/// Replace the field's window procedure so it can capture a combination.
unsafe extern "system" fn hotkey_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let key = (wparam.0 & 0xFFFF) as u16;
            if is_modifier_key(key) {
                // A modifier on its own is not a combination; wait for a real key.
                return LRESULT(0);
            }

            let modifiers = held_modifiers();
            if !modifiers.any() {
                // Refused, and said so: registered system-wide this would swallow
                // the key from every other program.
                if let Some(hint) = with_state(|state| state.hotkey_hint) {
                    let _ = set_text(hint, HOTKEY_NEEDS_MODIFIER);
                }
                return LRESULT(0);
            }

            let hotkey = Hotkey { modifiers, key };
            with_state(|state| state.hotkey = Some(hotkey));
            if let Some(field) = with_state(|state| state.hotkey_field) {
                let _ = set_text(field, &hotkey.to_string());
            }
            if let Some(hint) = with_state(|state| state.hotkey_hint) {
                let _ = set_text(hint, HOTKEY_HINT);
            }
            LRESULT(0)
        }

        _ => {
            let previous = with_state(|state| state.previous_proc).unwrap_or(0);
            let previous: WNDPROC = unsafe { std::mem::transmute(previous) };
            unsafe { CallWindowProcW(previous, hwnd, msg, wparam, lparam) }
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_COMMAND => {
            match (wparam.0 & 0xFFFF) as i32 {
                ID_BROWSE => browse_for_folder(),
                ID_TRAY_CLICK => toggle_tray_click(),
                ID_SAVE => save(),
                ID_CANCEL => finish(None),
                _ => {}
            }
            LRESULT(0)
        }

        WM_CLOSE => {
            finish(None);
            LRESULT(0)
        }

        WM_DESTROY => {
            with_state(|state| state.done = true);
            LRESULT(0)
        }

        WM_NCDESTROY => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },

        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Ask for a folder and put it in the field.
fn browse_for_folder() {
    let Some(field) = with_state(|state| state.output_dir) else {
        return;
    };
    let owner = with_state(|state| state.window).unwrap_or_default();

    let mut display = [0u16; 260];
    let browse = BROWSEINFOW {
        hwndOwner: owner,
        pszDisplayName: windows::core::PWSTR(display.as_mut_ptr()),
        lpszTitle: w!("Choose where captures are saved"),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_EDITBOX | BIF_NEWDIALOGSTYLE,
        ..Default::default()
    };

    let pidl = unsafe { SHBrowseForFolderW(&browse) };
    if pidl.is_null() {
        // Cancelled, which is not a failure.
        return;
    }

    let mut path = [0u16; 260];
    let copied = unsafe { SHGetPathFromIDListW(pidl, &mut path) };
    unsafe {
        ILFree(Some(pidl as *const _));
    }
    if !copied.as_bool() {
        return;
    }

    let text = String::from_utf16_lossy(&path[..path.iter().position(|c| *c == 0).unwrap_or(0)]);
    let _ = set_text(field, &text);
}

/// Swap what a left click on the tray icon does.
fn toggle_tray_click() {
    let Some(choice) = with_state(|state| {
        state.tray_left_click = state.tray_left_click.toggled();
        state.tray_left_click
    }) else {
        return;
    };
    if let Some(button) = with_state(|state| state.tray_click) {
        let _ = set_text(button, &tray_click_text(choice));
    }
}

/// Collect the controls, refuse an incomplete window, and close with a result.
fn save() {
    let Some(output_dir) = with_state(|state| text_of(state.output_dir).trim().to_string()) else {
        return;
    };

    if output_dir.is_empty() {
        if let Some(hint) = with_state(|state| state.output_hint) {
            let _ = set_text(hint, OUTPUT_NEEDS_FOLDER);
        }
        return;
    }

    let collected = with_state(|state| {
        let updated = state.current.clone();
        let hotkey = state.hotkey;
        let tray_left_click = state.tray_left_click;
        (updated, hotkey, tray_left_click)
    });
    let Some((mut updated, hotkey, tray_left_click)) = collected else {
        return;
    };

    let Some(hotkey) = hotkey else {
        if let Some(hint) = with_state(|state| state.hotkey_hint) {
            let _ = set_text(hint, HOTKEY_NEEDS_MODIFIER);
        }
        return;
    };

    updated.hotkey = hotkey;
    updated.output_dir = output_dir;
    updated.tray_left_click = tray_left_click;
    updated.clipboard = with_state(|state| is_checked(state.clipboard)).unwrap_or(false);
    updated.save_file = with_state(|state| is_checked(state.save_file)).unwrap_or(false);

    finish(Some(updated));
}

/// Record the outcome and take the window down.
fn finish(result: Option<Settings>) {
    let window = with_state(|state| {
        state.result = result;
        state.window
    });
    if let Some(window) = window {
        if !window.is_invalid() {
            unsafe {
                let _ = DestroyWindow(window);
            }
        }
    }
    with_state(|state| state.done = true);
}

/// Centre the window on the screen, or over the owner when it has a size.
fn center(window: HWND, owner: HWND) {
    unsafe {
        let mut rect = Default::default();
        if GetWindowRect(window, &mut rect).is_err() {
            return;
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let mut x = (GetSystemMetrics(SM_CXSCREEN) - width) / 2;
        let mut y = (GetSystemMetrics(SM_CYSCREEN) - height) / 2;

        let mut owner_rect = Default::default();
        if !owner.is_invalid() && GetWindowRect(owner, &mut owner_rect).is_ok() {
            let owner_width = owner_rect.right - owner_rect.left;
            if owner_width > 0 {
                x = owner_rect.left + (owner_width - width) / 2;
                y = owner_rect.top + (owner_rect.bottom - owner_rect.top - height) / 2;
            }
        }

        let _ = SetWindowPos(
            window,
            Some(HWND_TOP),
            x,
            y,
            width,
            height,
            SWP_NOZORDER,
        );
    }
}

/// Read a checkbox.
fn is_checked(control: HWND) -> bool {
    let state = unsafe { SendMessageW(control, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))) };
    state.0 == BST_CHECKED
}

/// Set a checkbox.
fn set_checked(control: HWND, checked: bool) {
    let value = if checked { BST_CHECKED } else { BST_UNCHECKED };
    unsafe {
        SendMessageW(control, BM_SETCHECK, Some(WPARAM(value as usize)), Some(LPARAM(0)));
    }
}

/// Read the text of a control.
fn text_of(control: HWND) -> String {
    let mut buffer = [0u16; 512];
    let length = unsafe { GetWindowTextW(control, &mut buffer) };
    if length <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..length as usize])
}

/// Replace the text of a control.
fn set_text(control: HWND, text: &str) -> Result<()> {
    let text = wide(text);
    unsafe { SetWindowTextW(control, PCWSTR::from_raw(text.as_ptr())) }
        .context("SetWindowTextW failed")
}

/// How the field reads.
fn hotkey_text(hotkey: Option<Hotkey>) -> String {
    match hotkey {
        Some(hotkey) => hotkey.to_string(),
        None => String::from("Click here and press a combination"),
    }
}

/// What the tray click button reads.
fn tray_click_text(choice: TrayClick) -> String {
    let action = match choice {
        TrayClick::Menu => "open the menu",
        TrayClick::Capture => "start a selection",
    };
    format!("Left click on the tray icon: {action}")
}

/// The modifiers held right now, as the combination builder needs them.
fn held_modifiers() -> Modifiers {
    let down = |key: i32| unsafe { GetKeyState(key) } < 0;
    Modifiers {
        control: down(VK_CONTROL.0 as i32),
        shift: down(VK_SHIFT.0 as i32),
        alt: down(VK_MENU.0 as i32),
        win: down(VK_LWIN.0 as i32) || down(VK_RWIN.0 as i32),
    }
}

/// Whether a virtual key is a modifier rather than the key of the combination.
fn is_modifier_key(key: u16) -> bool {
    [
        VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN, VK_LSHIFT, VK_RSHIFT, VK_RCONTROL,
        VK_LMENU, VK_RMENU,
    ]
    .iter()
    .any(|modifier| modifier.0 == key)
}

/// Encode text as the null-terminated wide string the Win32 calls expect.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
