//! The settings window.
//!
//! Opened from the resident mode's menu. It is modal in practice: the tray window
//! is disabled while it is up and a nested message loop runs until Save or
//! Cancel, so the caller receives the new settings or nothing at all.
//!
//! Four decisions keep this small.
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
//! Every coordinate comes from `ui_layout::Layout`, and the window is built again
//! whenever the display it sits on changes scale. The process is per-monitor DPI
//! aware, so Windows does not scale this window: without that it would be drawn at
//! its 96-DPI size on a 150% display, too small for its own contents.
//!
//! Nothing here writes anything. It hands the edited settings back and the
//! resident mode decides what to do with them, which keeps saving the file and
//! re-registering the hotkey in one place.

use std::cell::RefCell;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CLIP_DEFAULT_PRECIS, COLOR_BTNFACE, COLOR_WINDOW, CreateCompatibleDC, CreateFontW,
    CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_GUI_FONT, DEFAULT_PITCH, DEFAULT_QUALITY, DeleteDC,
    DeleteObject, FF_DONTCARE, GetObjectW, GetStockObject, GetSysColor, GetTextExtentPoint32W, HBRUSH,
    HDC, HGDIOBJ, InvalidateRect, LOGFONTW, NULL_BRUSH, OUT_DEFAULT_PRECIS, RDW_ALLCHILDREN,
    RDW_ERASE, RDW_INVALIDATE, RDW_UPDATENOW, RedrawWindow, SelectObject, SetBkMode, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, SetFocus, VK_CONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
    VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::Shell::{
    BIF_EDITBOX, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, ILFree, SHBrowseForFolderW,
    SHGetPathFromIDListW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BeginDeferWindowPos, CallWindowProcW, CreateWindowExW, DefWindowProcW,
    DeferWindowPos, DestroyWindow, DispatchMessageW, EndDeferWindowPos, GWLP_WNDPROC, GetMessageW,
    GetClassNameW, GetClientRect, GetSystemMetrics, GetWindowRect, GetWindowTextW, HDWP, HMENU,
    HWND_TOP, IsDialogMessageW, MSG,
    RegisterClassW, SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_SHOW, SW_SHOWNA, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOZORDER, SendMessageW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
    SetWindowTextW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE,
    WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_KEYDOWN, WM_NCDESTROY, WM_SETFONT,
    WM_SYSKEYDOWN,
    WNDCLASSW, WNDPROC, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE,
    WS_EX_DLGMODALFRAME, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::hotkey::{Hotkey, Modifiers};
use crate::settings::{Settings, TrayClick};
use crate::ui_layout::{self, Layout, Rect};

/// Control styles and messages whose values come from the Win32 headers.
///
/// They are spelled out because the crate models several of them as newtypes over
/// the same integers while `CreateWindowExW` wants plain style bits, and mixing
/// the two is noise for a handful of constants. Every value is the one the
/// headers define.
const BS_AUTOCHECKBOX: u32 = 0x0000_0003;
const BS_GROUPBOX: u32 = 0x0000_0007;
const BS_PUSHBUTTON: u32 = 0x0000_0000;
const BS_DEFPUSHBUTTON: u32 = 0x0000_0001;
const ES_AUTOHSCROLL: u32 = 0x0000_0080;
const ES_READONLY: u32 = 0x0000_0800;
const SS_LEFT: u32 = 0x0000_0000;
const SS_RIGHT: u32 = 0x0000_0002;
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const BST_CHECKED: isize = 1;
const BST_UNCHECKED: isize = 0;

/// The window has a caption and a close button, and is not resizable: its size
/// follows from the layout, so there is nothing for a resize grip to do.
///
/// `WS_CLIPCHILDREN` keeps the window from painting over its own controls. A
/// control is a window in its own right and draws itself, so a parent that paints
/// the whole client area on every repaint would erase its controls and leave them
/// to draw themselves again afterwards.
const WINDOW_STYLE_BITS: u32 = WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_CLIPCHILDREN.0;
const WINDOW_EX_STYLE_BITS: WINDOW_EX_STYLE = WS_EX_DLGMODALFRAME;

fn window_style() -> WINDOW_STYLE {
    WINDOW_STYLE(WINDOW_STYLE_BITS)
}

/// What each group frame is called.
///
/// The frames are what tell the eye that the shortcut, the destinations and the
/// tray icon are separate concerns. As one flat column of controls the window
/// read as a list of unrelated lines.
const GROUP_SHORTCUT: &str = "Shortcut";
const GROUP_AFTER: &str = "After each capture";
const GROUP_FILE: &str = "Saving to a file";
const GROUP_TRAY: &str = "Tray icon";

/// What the window says.
const HOTKEY_HINT: &str = "Click the field and press a combination. A modifier is required.";
const HOTKEY_NEEDS_MODIFIER: &str =
    "Refused: without a modifier that key would be taken from every other program.";
const OUTPUT_HINT: &str =
    "Only used when saving to a file. Names carry the time the capture was taken.";
const OUTPUT_NEEDS_FOLDER: &str = "Refused: choose an output folder before saving.";
const TRAY_CLICK_HINT: &str = "The other mouse button always starts a selection.";

/// The labels of the three buttons. The layout sizes a button from its own text,
/// so these are the strings that decide how wide it is.
const BROWSE_TEXT: &str = "Browse...";
const SAVE_TEXT: &str = "Save";
const CANCEL_TEXT: &str = "Cancel";

/// Identifiers, so a message can say which control it came from.
const ID_HOTKEY: i32 = 100;
const ID_HOTKEY_HINT: i32 = 101;
const ID_CLIPBOARD: i32 = 102;
const ID_SAVE_FILE: i32 = 103;
const ID_CURSOR: i32 = 109;
const ID_OUTPUT_DIR: i32 = 104;
const ID_BROWSE: i32 = 105;
const ID_OUTPUT_HINT: i32 = 106;
const ID_TRAY_CLICK: i32 = 107;
const ID_TRAY_CLICK_HINT: i32 = 108;
/// The two buttons use the identifiers Windows treats as OK and Cancel, so the
/// standard dialog keyboard handling — Enter on the default button, Escape to
/// close — produces the messages this window already understands.
const ID_SAVE: i32 = 1;
const ID_CANCEL: i32 = 2;

/// Every control the layout places.
///
/// They are held in one value so a DPI change can move them without rebuilding
/// the window, and so `in_order` is the single place that knows the order they
/// have to be created and restacked in.
#[derive(Default, Clone, Copy)]
struct Controls {
    shortcut_group: HWND,
    hotkey_label: HWND,
    hotkey_field: HWND,
    hotkey_hint: HWND,

    after_group: HWND,
    clipboard: HWND,
    cursor: HWND,
    save_file: HWND,

    file_group: HWND,
    output_label: HWND,
    output_dir: HWND,
    browse: HWND,
    output_hint: HWND,

    tray_group: HWND,
    tray_click: HWND,
    tray_click_hint: HWND,

    save: HWND,
    cancel: HWND,
}

impl Controls {
    /// Every control that exists, group frames first.
    ///
    /// The frames come first because a group box is painted as a frame with a gap
    /// in it for its caption: a frame created after the controls it contains is
    /// drawn over them, and its caption lands on top of the first row inside it.
    fn in_order(&self) -> Vec<HWND> {
        [
            self.shortcut_group,
            self.after_group,
            self.file_group,
            self.tray_group,
            self.hotkey_label,
            self.hotkey_field,
            self.hotkey_hint,
            self.clipboard,
            self.cursor,
            self.save_file,
            self.output_label,
            self.output_dir,
            self.browse,
            self.output_hint,
            self.tray_click,
            self.tray_click_hint,
            self.save,
            self.cancel,
        ]
        .into_iter()
        .filter(|handle| !handle.is_invalid())
        .collect()
    }
}

struct State {
    window: HWND,
    owner: HWND,
    instance: HINSTANCE,
    controls: Controls,
    /// The font every control is drawn with, sized for the display the window is
    /// on. Owned here, so it is released exactly once.
    font: HGDIOBJ,
    /// The brush the group frames are given, built from the same system colour the
    /// window class names as its background. A frame is a button, and this is what
    /// it fills the gap behind its caption with. Owned here, so a frame repainting
    /// does not create a brush every time.
    page_brush: HBRUSH,
    /// The brush the fields are painted with: `COLOR_WINDOW`, which is white, so a
    /// field reads as somewhere to type rather than as part of the page.
    window_brush: HBRUSH,
    layout: Layout,
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
    /// Whether the window has been shown yet. A scale change before that only has
    /// to move controls; afterwards it has to replace them, because the window is
    /// already drawn and its old contents are on the screen.
    visible: bool,
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
///
/// `dpi` overrides the scale the layout is built for. Only `lasso settings --dpi`
/// passes one: the layout scales with the display it opens on, and the override is
/// how a scale can be checked without a second monitor to check it on.
pub fn open(owner: HWND, current: &Settings, dpi: Option<u32>) -> Result<Option<Settings>> {
    let explicit = dpi;
    let instance = HINSTANCE(
        unsafe { GetModuleHandleW(None) }
            .context("GetModuleHandleW failed")?
            .0,
    );

    let class = w!("LassoSettings");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class,
        // The window has no background of its own to draw, so Windows erases it
        // with the button face its controls are drawn on.
        //
        // This is not decoration. With no brush there is nothing to erase with, so
        // a window that grows keeps whatever was on the screen in the area it grew
        // into, and every control that moves leaves its old contents behind on a
        // background that was never repainted. Both are invisible at the size the
        // window opens at and unmissable after a change of display scale.
        hbrBackground: HBRUSH((COLOR_BTNFACE.0 as usize + 1) as *mut _),
        ..Default::default()
    };
    // A zero return also means this process already registered the class, which is
    // what a second visit to the settings looks like.
    unsafe {
        let _ = RegisterClassW(&wc);
    }

    // There is no window yet, so the display the window will open on is the one
    // the system reports for the process. `lasso settings` runs without a tray
    // window, so the owner is not a window to ask either.
    let dpi = dpi.unwrap_or_else(|| unsafe { GetDpiForSystem() });
    // The font comes first because the buttons are measured from it: the layout
    // cannot be built without knowing how wide its own labels are.
    let font = create_font(ui_layout::factor(dpi));
    let layout = layout_for(dpi, font);

    // The brush a group frame fills the gap behind its caption with, from the same
    // system colour the window class names as its background. Labels are handed no
    // brush at all, which is a decision rather than an omission: see
    // `WM_CTLCOLORSTATIC`.
    let page_brush = unsafe { CreateSolidBrush(COLORREF(GetSysColor(COLOR_BTNFACE))) };
    let window_brush = unsafe { CreateSolidBrush(COLORREF(GetSysColor(COLOR_WINDOW))) };

    // The size given to CreateWindowExW covers the whole window, caption and
    // frame included, while the layout is written in client coordinates. Turning
    // one into the other here is what keeps the bottom row of buttons inside the
    // window instead of below its edge.
    let frame = window_frame(layout);

    let created = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE_BITS,
            class,
            w!("lasso settings"),
            window_style(),
            0,
            0,
            frame.right - frame.left,
            frame.bottom - frame.top,
            Some(owner),
            None,
            Some(instance),
            None,
        )
    };
    // No state was installed yet, so this is the one failure that has to release
    // what was built for the window itself.
    let window = match created {
        Ok(window) => window,
        Err(err) => {
            unsafe {
                let _ = DeleteObject(font);
                let _ = DeleteObject(HGDIOBJ(page_brush.0));
                let _ = DeleteObject(HGDIOBJ(window_brush.0));
            }
            return Err(err).context("could not create the settings window");
        }
    };

    // The state is installed before anything can go wrong, so a failure below can
    // always tear the window down through the same path.
    STATE.with(|slot| {
        *slot.borrow_mut() = Some(State {
            window,
            owner,
            instance,
            controls: Controls::default(),
            font,
            page_brush,
            window_brush,
            layout,
            hotkey: Some(current.hotkey),
            tray_left_click: current.tray_left_click,
            current: current.clone(),
            previous_proc: 0,
            visible: false,
            done: false,
            result: None,
        });
    });

    if let Err(err) = build_controls() {
        abandon(window);
        return Err(err);
    }

    // The window is measured and corrected, rather than trusted to match what it
    // was asked for. The frame around a client area is not this program's to
    // compute: `AdjustWindowRectExForDpi` returns a window size whose client area
    // is a few pixels off at some scales, in one direction or the other, and every
    // control in the layout is placed against a client width that then does not
    // exist. The visible result is a control sized to the layout's last column
    // reaching past the frame it is supposed to sit inside.
    //
    // Taking the difference from the window itself settles it whatever the frame
    // is made of, and at 100% the difference is nothing, so this costs one call.
    match_exact_client(window);

    // The layout was built before the window existed, from the DPI the system
    // reported. Asking the window itself settles any difference - the display it
    // actually opened on may not be the one the process default describes - before
    // anything is drawn at the wrong size. An explicit scale is left alone, since
    // the point of asking for one is to see the layout built at it.
    if explicit.is_none() {
        let open_dpi = unsafe { GetDpiForWindow(window) };
        if open_dpi != dpi {
            rescale(open_dpi);
        }
    }
    load_values();

    center(window, owner);
    unsafe {
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(window, SW_SHOW);
        let _ = SetForegroundWindow(window);
        if let Some(field) = with_state(|state| state.controls.hotkey_field) {
            let _ = SetFocus(Some(field));
        }
    }
    // From here on a scale change has to replace the controls rather than move
    // them, so the window has something of its own on screen to keep in step.
    with_state(|state| state.visible = true);
    trace_layout();

    // A nested loop rather than a modal dialog box: the resident mode already runs
    // an ordinary message pump and this runs inside it. `WM_QUIT` is deliberately
    // never posted here, because that would end the resident loop too; the loop
    // ends on the flag the window sets when it is destroyed.
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
        // Tab between the controls, Enter on the default button and Escape to
        // close all come from here: a plain window has none of them by itself,
        // and a settings window without them feels wrong to anyone who reaches
        // for the keyboard.
        let handled = unsafe { IsDialogMessageW(window, &message) }.as_bool();
        if !handled {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if with_state(|state| state.done).unwrap_or(true) {
            break;
        }
    }

    let result = with_state(|state| state.result.take()).flatten();
    let owner = with_state(|state| state.owner).unwrap_or_default();
    abandon(window);

    if !owner.is_invalid() {
        unsafe {
            let _ = EnableWindow(owner, true);
        }
    }
    Ok(result)
}

/// Shut the window down and forget the state.
///
/// Every path out of `open` goes through here, so a failure while building the
/// window cannot leave a font behind, and a window that closed itself cannot
/// leave `open` waiting on a message loop that will never end.
fn abandon(window: HWND) {
    let (font, page_brush, window_brush) =
        with_state(|state| (state.font, state.page_brush, state.window_brush)).unwrap_or_default();
    if !font.is_invalid() {
        unsafe {
            let _ = DeleteObject(font);
        }
    }
    for brush in [page_brush, window_brush] {
        if !brush.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(brush.0));
            }
        }
    }
    if !window.is_invalid() {
        unsafe {
            let _ = DestroyWindow(window);
        }
    }
    STATE.with(|slot| *slot.borrow_mut() = None);
}

/// The layout for a display, with its buttons measured from the font.
fn layout_for(dpi: u32, font: HGDIOBJ) -> Layout {
    let factor = ui_layout::factor(dpi);
    // Each button is as wide as the widest of the labels it carries, so a
    // translation cannot be clipped. The pair on the right shares one width, which
    // is what makes them a pair rather than two buttons that happen to be near
    // each other.
    let browse = ui_layout::button_width(measure_text(font, BROWSE_TEXT, factor), factor);
    let pair = SAVE_TEXT.chars().count().max(CANCEL_TEXT.chars().count()) as i32 * 6;
    Layout::new(dpi, browse, ui_layout::button_width(pair, factor))
}

/// How wide a string is in a font, in 96-DPI pixels.
///
/// The font is created per display, so it is measured at the scale in use and the
/// result is brought back to the units `Layout` works in. Measuring at the base
/// scale instead would mean creating a second font to measure with.
///
/// A memory device context is enough to ask a font for its metrics: nothing is
/// drawn, and the context is released before returning.
fn measure_text(font: HGDIOBJ, text: &str, factor: f64) -> i32 {
    // A per-character estimate is what answers when there is no font to measure:
    // buttons that are roughly right beat buttons collapsed to nothing.
    let estimate = || (6 * text.chars().count() as i32).max(1);
    if font.is_invalid() {
        return estimate();
    }

    let mut size = SIZE::default();
    let measured = unsafe {
        let context = CreateCompatibleDC(None);
        if context.is_invalid() {
            return estimate();
        }
        let previous = SelectObject(context, font);
        let text = wide(text);
        // The terminator is not part of a text extent, so the slice stops short
        // of it.
        let width = GetTextExtentPoint32W(context, &text[..text.len() - 1], &mut size);
        SelectObject(context, previous);
        let _ = DeleteDC(context);
        width.as_bool()
    };

    if !measured || size.cx <= 0 {
        return estimate();
    }
    // `scale` is the inverse of itself for a positive factor, which is what makes
    // this the way back to base units.
    ui_layout::scale(size.cx, 1.0 / factor).max(1)
}

/// The window size that gives the layout its client area.
///
/// The frame has to be asked for at the **display's own scale**, not at the
/// process default's. `AdjustWindowRectEx` is scale-dependent and reads the
/// thread's awareness: the process is per-monitor aware, so it works in physical
/// pixels, while the client size it is handed here has already been scaled by the
/// layout. Asking the scale-free version therefore scales the frame a second time
/// and returns a window whose client area is a few pixels short of the one the
/// layout was built for — which is invisible until something sized to the
/// layout's width reaches past the frame it is supposed to sit inside.
fn window_frame(layout: Layout) -> RECT {
    let mut frame = RECT {
        left: 0,
        top: 0,
        right: layout.client_width,
        bottom: layout.client_height,
    };
    unsafe {
        let _ = AdjustWindowRectExForDpi(
            &mut frame,
            window_style(),
            false,
            WINDOW_EX_STYLE_BITS,
            layout.dpi,
        );
    }
    frame
}

/// The description of a font, in the same shape `CreateFontW` takes.
fn font_metrics(font: HGDIOBJ) -> Option<LOGFONTW> {
    if font.is_invalid() {
        return None;
    }
    let mut info = LOGFONTW::default();
    let measured = unsafe {
        GetObjectW(
            font,
            std::mem::size_of::<LOGFONTW>() as i32,
            Some(&mut info as *mut _ as *mut _),
        )
    };
    if measured <= 0 || info.lfHeight == 0 {
        return None;
    }
    Some(info)
}

/// Create the font for the current display.
///
/// The stock GUI font is one shared object drawn at 96 DPI, so it does not follow
/// a window to a denser display. A private font built from its face name does,
/// and that is what keeps the text the same size relative to the controls as the
/// controls are to each other.
fn create_font(factor: f64) -> HGDIOBJ {
    let stock = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    let Some(info) = font_metrics(stock) else {
        // Nothing to copy. Keeping the stock font leaves the window readable,
        // which is better than refusing to open it.
        return stock;
    };

    // The negative height asks for a character height rather than a cell height,
    // which is what the value measured from the stock font already is.
    let height = ui_layout::scale(info.lfHeight.abs(), factor).max(1);
    let font = unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            info.lfWeight,
            u32::from(info.lfItalic),
            u32::from(info.lfUnderline),
            u32::from(info.lfStrikeOut),
            if info.lfCharSet.0 == 0 {
                DEFAULT_CHARSET
            } else {
                info.lfCharSet
            },
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            DEFAULT_QUALITY,
            u32::from(DEFAULT_PITCH.0) | u32::from(FF_DONTCARE.0),
            PCWSTR::from_raw(info.lfFaceName.as_ptr()),
        )
    };
    if font.is_invalid() {
        stock
    } else {
        HGDIOBJ(font.0)
    }
}

/// Create every control for the current layout.
fn build_controls() -> Result<()> {
    let Some((window, instance, layout)) =
        with_state(|state| (state.window, state.instance, state.layout))
    else {
        return Ok(());
    };

    // The frames first: a group box is a frame with a gap for its caption, and one
    // created after the controls inside it is painted over them.
    let shortcut_group = group(
        window,
        GROUP_SHORTCUT,
        layout.group(layout.shortcut_top, layout.shortcut_height),
        instance,
    )?;
    let after_group = group(
        window,
        GROUP_AFTER,
        layout.group(layout.after_top, layout.after_height),
        instance,
    )?;
    let file_group = group(
        window,
        GROUP_FILE,
        layout.group(layout.file_top, layout.file_height),
        instance,
    )?;
    let tray_group = group(
        window,
        GROUP_TRAY,
        layout.group(layout.tray_top, layout.tray_height),
        instance,
    )?;

    let top = layout.content_top(layout.shortcut_top);
    let hotkey_label = child(
        window,
        w!("STATIC"),
        "Hotkey",
        SS_RIGHT,
        Default::default(),
        -1,
        layout.label(top),
        instance,
    )?;
    let hotkey_field = child(
        window,
        w!("EDIT"),
        "",
        ES_READONLY | ES_AUTOHSCROLL,
        WS_EX_CLIENTEDGE,
        ID_HOTKEY,
        layout.field(top, layout.field_width),
        instance,
    )?;
    let hotkey_hint = child(
        window,
        w!("STATIC"),
        HOTKEY_HINT,
        SS_LEFT,
        Default::default(),
        ID_HOTKEY_HINT,
        layout.hint(top),
        instance,
    )?;

    let rects = layout.checks();
    let (clipboard_rect, cursor_rect, save_file_rect) = (rects[0], rects[1], rects[2]);
    let clipboard = child(
        window,
        w!("BUTTON"),
        "Copy each capture to the clipboard",
        BS_AUTOCHECKBOX | WS_TABSTOP.0,
        Default::default(),
        ID_CLIPBOARD,
        clipboard_rect,
        instance,
    )?;
    let cursor = child(
        window,
        w!("BUTTON"),
        "Draw the mouse pointer into the capture",
        BS_AUTOCHECKBOX | WS_TABSTOP.0,
        Default::default(),
        ID_CURSOR,
        cursor_rect,
        instance,
    )?;
    let save_file = child(
        window,
        w!("BUTTON"),
        "Also save each capture to a file",
        BS_AUTOCHECKBOX | WS_TABSTOP.0,
        Default::default(),
        ID_SAVE_FILE,
        save_file_rect,
        instance,
    )?;

    let (output_label_rect, output_dir_rect, browse_rect) = layout.output();
    let output_label = child(
        window,
        w!("STATIC"),
        "Output folder",
        SS_RIGHT,
        Default::default(),
        -1,
        output_label_rect,
        instance,
    )?;
    let output_dir = child(
        window,
        w!("EDIT"),
        "",
        ES_AUTOHSCROLL | WS_TABSTOP.0,
        WS_EX_CLIENTEDGE,
        ID_OUTPUT_DIR,
        output_dir_rect,
        instance,
    )?;
    let browse = child(
        window,
        w!("BUTTON"),
        BROWSE_TEXT,
        BS_PUSHBUTTON | WS_TABSTOP.0,
        Default::default(),
        ID_BROWSE,
        browse_rect,
        instance,
    )?;
    let output_hint = child(
        window,
        w!("STATIC"),
        OUTPUT_HINT,
        SS_LEFT,
        Default::default(),
        ID_OUTPUT_HINT,
        layout.hint(layout.content_top(layout.file_top)),
        instance,
    )?;

    let (tray_rect, tray_hint_rect) = layout.tray();
    let tray_click = child(
        window,
        w!("BUTTON"),
        "",
        BS_PUSHBUTTON | WS_TABSTOP.0,
        Default::default(),
        ID_TRAY_CLICK,
        tray_rect,
        instance,
    )?;
    let tray_click_hint = child(
        window,
        w!("STATIC"),
        TRAY_CLICK_HINT,
        SS_LEFT,
        Default::default(),
        ID_TRAY_CLICK_HINT,
        tray_hint_rect,
        instance,
    )?;

    let (save_rect, cancel_rect) = layout.buttons();
    let save = child(
        window,
        w!("BUTTON"),
        SAVE_TEXT,
        BS_DEFPUSHBUTTON | WS_TABSTOP.0,
        Default::default(),
        ID_SAVE,
        save_rect,
        instance,
    )?;
    let cancel = child(
        window,
        w!("BUTTON"),
        CANCEL_TEXT,
        BS_PUSHBUTTON | WS_TABSTOP.0,
        Default::default(),
        ID_CANCEL,
        cancel_rect,
        instance,
    )?;

    // Capture keys on the field itself. A read-only edit still receives them.
    // The function is cast through a pointer rather than straight to an integer,
    // which is what the language requires, and what it replaces is kept: a control
    // has a window procedure of its own, and one that this module does not forward
    // to stops being a control. That is what an empty field and a field whose text
    // never changes both look like from the outside.
    let previous = unsafe {
        SetWindowLongPtrW(
            hotkey_field,
            GWLP_WNDPROC,
            hotkey_proc as *const () as isize,
        )
    };

    with_state(|state| {
        state.controls = Controls {
            shortcut_group,
            hotkey_label,
            hotkey_field,
            hotkey_hint,
            after_group,
            clipboard,
            cursor,
            save_file,
            file_group,
            output_label,
            output_dir,
            browse,
            output_hint,
            tray_group,
            tray_click,
            tray_click_hint,
            save,
            cancel,
        };
        state.previous_proc = previous;
    });
    Ok(())
}

/// Create a group frame.
///
/// A frame wants no extended style of its own, which is what it says here rather
/// than a second factory function for the one caller.
fn group(parent: HWND, caption: &str, rect: Rect, instance: HINSTANCE) -> Result<HWND> {
    child(
        parent,
        w!("BUTTON"),
        caption,
        BS_GROUPBOX,
        Default::default(),
        -1,
        rect,
        instance,
    )
}

/// Create one child control.
fn child(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    style: u32,
    ex_style: WINDOW_EX_STYLE,
    id: i32,
    rect: Rect,
    instance: HINSTANCE,
) -> Result<HWND> {
    let text = wide(text);
    let control = unsafe {
        CreateWindowExW(
            ex_style,
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
    let font = with_state(|state| state.font).unwrap_or_default();
    apply_font(control, font);
    Ok(control)
}

/// Put a font on a control, when there is one to put.
fn apply_font(control: HWND, font: HGDIOBJ) {
    if control.is_invalid() || font.is_invalid() {
        return;
    }
    unsafe {
        SendMessageW(
            control,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
    }
}

/// Move and resize every control for a layout.
///
/// The whole set goes through one `DeferWindowPos` batch. Moving them one at a
/// time redraws the window between each of them, so the frames and the controls
/// inside them are visible apart from each other for as long as the move takes —
/// and after a DPI change that is every control in the window.
fn place_controls(layout: Layout, controls: Controls) {
    let handles = controls.in_order();
    if handles.is_empty() {
        return;
    }

    let Ok(mut batch) = (unsafe { BeginDeferWindowPos(handles.len() as i32) }) else {
        return;
    };

    // A failed `DeferWindowPos` leaves the batch usable and the control where it
    // was, so a failure here is not worth abandoning the rest of the move for.
    let move_to = |batch: HDWP, control: HWND, rect: Rect| {
        unsafe {
            DeferWindowPos(
                batch,
                control,
                None,
                rect.0,
                rect.1,
                rect.2,
                rect.3,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        }
        .unwrap_or(batch)
    };

    batch = move_to(
        batch,
        controls.shortcut_group,
        layout.group(layout.shortcut_top, layout.shortcut_height),
    );
    batch = move_to(
        batch,
        controls.after_group,
        layout.group(layout.after_top, layout.after_height),
    );
    batch = move_to(
        batch,
        controls.file_group,
        layout.group(layout.file_top, layout.file_height),
    );
    batch = move_to(
        batch,
        controls.tray_group,
        layout.group(layout.tray_top, layout.tray_height),
    );

    let top = layout.content_top(layout.shortcut_top);
    batch = move_to(batch, controls.hotkey_label, layout.label(top));
    batch = move_to(
        batch,
        controls.hotkey_field,
        layout.field(top, layout.field_width),
    );
    batch = move_to(batch, controls.hotkey_hint, layout.hint(top));

    let rects = layout.checks();
    batch = move_to(batch, controls.clipboard, rects[0]);
    batch = move_to(batch, controls.cursor, rects[1]);
    batch = move_to(batch, controls.save_file, rects[2]);

    let (output_label, output_dir, browse) = layout.output();
    batch = move_to(batch, controls.output_label, output_label);
    batch = move_to(batch, controls.output_dir, output_dir);
    batch = move_to(batch, controls.browse, browse);
    batch = move_to(
        batch,
        controls.output_hint,
        layout.hint(layout.content_top(layout.file_top)),
    );

    let (tray, tray_hint) = layout.tray();
    batch = move_to(batch, controls.tray_click, tray);
    batch = move_to(batch, controls.tray_click_hint, tray_hint);

    let (save, cancel) = layout.buttons();
    batch = move_to(batch, controls.save, save);
    batch = move_to(batch, controls.cancel, cancel);

    unsafe {
        let _ = EndDeferWindowPos(batch);
    }

    // Moving a control is not the same as repainting it. `DeferWindowPos` leaves
    // each control alone until something asks it to redraw, and the window paints
    // its background around its children rather than over them, so a control that
    // is moved and not invalidated keeps its old contents on the parent's
    // background. Invalidating each one after the move is what stops the previous
    // layout from showing through the new one.
    for handle in handles {
        unsafe {
            let _ = InvalidateRect(Some(handle), None, true);
        }
    }
}

/// What the controls are showing, so a window that is rebuilt keeps it.
///
/// A scale change creates the controls again, and a window that dropped what the
/// user had typed would be worse than one that never scaled. Reading it out first
/// and writing it back afterwards is what carries unsaved edits across.
struct Held {
    hotkey: String,
    output_dir: String,
    tray_click: String,
    clipboard: bool,
    cursor: bool,
    save_file: bool,
}

impl Held {
    /// Read what every control holds.
    ///
    /// Every value is read out and written back in a single pass over the state.
    /// Reading a control sends it a message, and a message can come back into this
    /// module - which is what `with_state` refuses a second borrow for. A read
    /// attempted from inside another read therefore returns nothing at all, and an
    /// empty field is indistinguishable from a field the user cleared.
    fn read() -> Self {
        let Some(controls) = with_state(|state| state.controls) else {
            return Held {
                hotkey: String::new(),
                output_dir: String::new(),
                tray_click: String::new(),
                clipboard: false,
                cursor: false,
                save_file: false,
            };
        };
        Held {
            hotkey: text_of(controls.hotkey_field),
            output_dir: text_of(controls.output_dir),
            tray_click: text_of(controls.tray_click),
            clipboard: is_checked(controls.clipboard),
            cursor: is_checked(controls.cursor),
            save_file: is_checked(controls.save_file),
        }
    }

    /// Write it back into whatever controls exist now.
    fn restore(&self) {
        let Some(controls) = with_state(|state| state.controls) else {
            return;
        };
        if !self.hotkey.is_empty() {
            let _ = set_text(controls.hotkey_field, &self.hotkey);
        }
        if !self.output_dir.is_empty() {
            let _ = set_text(controls.output_dir, &self.output_dir);
        }
        if !self.tray_click.is_empty() {
            let _ = set_text(controls.tray_click, &self.tray_click);
        }
        set_checked(controls.clipboard, self.clipboard);
        set_checked(controls.cursor, self.cursor);
        set_checked(controls.save_file, self.save_file);
    }
}

/// Rebuild the window for a new display scale.
///
/// Does nothing when the scale has not actually changed, so a notification that
/// repeats the current value does not throw the font away for nothing.
///
/// Two paths, because the window can be told about a new scale in two situations.
/// Before it is shown - which is what happens when it opens on a display whose
/// scale the process default did not describe - there is nothing on screen to get
/// wrong, so the controls are simply moved. After it is shown, they are destroyed
/// and created again: moving a window that is already drawn leaves its new area
/// unpainted and its old contents on the screen, and rebuilding is both shorter
/// and the only version that cannot leave the two layouts mixed together. Every
/// control here is stateless, so nothing is lost by creating it again.
fn rescale(new_dpi: u32) {
    let Some(current) = with_state(|state| state.layout.dpi) else {
        return;
    };
    if current == new_dpi {
        return;
    }

    let font = create_font(ui_layout::factor(new_dpi));
    let layout = layout_for(new_dpi, font);

    let Some((window, visible, controls, previous_font)) = with_state(|state| {
        let previous = std::mem::replace(&mut state.font, font);
        state.layout = layout;
        (state.window, state.visible, state.controls, previous)
    }) else {
        return;
    };


    // Nothing may be drawn with the old font after it is released.
    for control in controls.in_order() {
        apply_font(control, font);
    }

    let frame = window_frame(layout);
    unsafe {
        let _ = SetWindowPos(
            window,
            Some(HWND_TOP),
            0,
            0,
            frame.right - frame.left,
            frame.bottom - frame.top,
            SWP_NOMOVE | SWP_NOZORDER,
        );
    }

    if !visible {
        // The window has not been shown, so its controls are empty and there is
        // nothing on screen to get wrong. Moving them is enough, and the caller
        // fills them from the saved values afterwards.
        place_controls(layout, controls);
        release_font(previous_font, font);
        return;
    }

    // From here the window is on screen, and its contents are the user's: what the
    // controls hold is read out first, because a change of display scale is not a
    // reason to lose an edit that has not been saved yet.
    let held = Held::read();

    // The window is erased once more with nothing on it, and only then given its
    // controls back.
    //
    // This is what a change of scale needs, and moving the existing controls is
    // not it. A window paints its own background around its children rather than
    // over them, so the pixels a control drew are never erased by anything that
    // happens to the window afterwards: a control that moves leaves its old
    // contents behind, and a window that grows keeps whatever was on the screen in
    // the area it grew into. New controls drawn over the old ones look right in a
    // list of controls and wrong on the screen.
    unsafe {
        let _ = ShowWindow(window, SW_HIDE);
    }
    for control in controls.in_order() {
        unsafe {
            let _ = DestroyWindow(control);
        }
    }
    with_state(|state| state.controls = Controls::default());

    // Nothing is left to clip against, so this erases the whole client area.
    unsafe {
        let _ = RedrawWindow(
            Some(window),
            None,
            None,
            RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
        );
    }

    if let Err(err) = build_controls() {
        // The window is unusable without its controls, so it is taken down rather
        // than shown in a state the user cannot finish.
        eprintln!("[settings] could not rebuild the window for {new_dpi} dpi: {err:#}");
        release_font(previous_font, font);
        finish(None);
        return;
    }
    unsafe {
        // Shown without taking the focus back: the window is already modal to its
        // owner and nothing here is a user action.
        let _ = ShowWindow(window, SW_SHOWNA);
    }
    held.restore();

    release_font(previous_font, font);
    clamp_to_screen(window);

    // The window's own background is painted outside its children, so a full
    // repaint is what clears the area the previous layout occupied.
    unsafe {
        let _ = RedrawWindow(
            Some(window),
            None,
            None,
            RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW,
        );
    }
}

/// Release the font the window was using, once nothing selects it.
fn release_font(previous: HGDIOBJ, current: HGDIOBJ) {
    if !previous.is_invalid() && previous != current {
        unsafe {
            let _ = DeleteObject(previous);
        }
    }
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
            state.current.cursor,
            state.current.save_file,
            state.tray_left_click,
            state.controls.hotkey_field,
            state.controls.hotkey_hint,
            state.controls.output_dir,
            state.controls.output_hint,
            state.controls.tray_click,
            state.controls.clipboard,
            state.controls.cursor,
            state.controls.save_file,
        )
    }) else {
        return;
    };
    let (
        hotkey,
        output_dir,
        clipboard_on,
        cursor_on,
        save_on,
        tray_left_click,
        hotkey_field,
        hotkey_hint,
        output_control,
        output_hint_control,
        tray_control,
        clipboard_control,
        cursor_control,
        save_control,
    ) = values;

    let _ = set_text(hotkey_field, &hotkey_text(hotkey));
    let _ = set_text(hotkey_hint, HOTKEY_HINT);
    let _ = set_text(output_control, &output_dir);
    let _ = set_text(output_hint_control, OUTPUT_HINT);
    let _ = set_text(tray_control, &tray_click_text(tray_left_click));
    set_checked(clipboard_control, clipboard_on);
    set_checked(cursor_control, cursor_on);
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
                if let Some(hint) = with_state(|state| state.controls.hotkey_hint) {
                    let _ = set_text(hint, HOTKEY_NEEDS_MODIFIER);
                }
                return LRESULT(0);
            }

            let hotkey = Hotkey { modifiers, key };
            with_state(|state| state.hotkey = Some(hotkey));
            if let Some(field) = with_state(|state| state.controls.hotkey_field) {
                let _ = set_text(field, &hotkey.to_string());
            }
            if let Some(hint) = with_state(|state| state.controls.hotkey_hint) {
                let _ = set_text(hint, HOTKEY_HINT);
            }
            LRESULT(0)
        }

        _ => {
            // Every message this module does not handle - which is all of them
            // except the two key messages above - goes back to the control's own
            // window procedure. Without it the field is not an edit control any
            // more: it does not store its text, does not draw it, and does not
            // take the caret.
            let previous = with_state(|state| state.previous_proc).unwrap_or(0);
            if previous == 0 {
                return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            }
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

        WM_DPICHANGED => {
            // The window is per-monitor DPI aware, so moving it to another display
            // does not scale it. This notification says the scale changed; the
            // frame Windows suggests for the new display is not used, because a
            // window whose size follows from its contents keeps the position it
            // has and is only brought back on screen.
            rescale((wparam.0 & 0xFFFF) as u32);
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

        // A group frame is a button, and a button is supposed to ask for its colour
        // here rather than through `WM_CTLCOLORSTATIC`. It is answered with the
        // page's own colour, and **the answer does not take**: measured against a
        // capture, the caption's background is `COLOR_WINDOW` white whether this
        // message is handled or falls through to `DefWindowProcW`. The themed group
        // box paints its caption itself. It is handled anyway, because it is the
        // documented answer and costs nothing, but it is not what fixes the
        // caption - nothing in this window does.
        WM_CTLCOLORBTN => {
            let page_brush = with_state(|state| state.page_brush).unwrap_or_default();
            if page_brush.is_invalid() {
                return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            }
            LRESULT(page_brush.0 as isize)
        }

        // A label and a field ask the same question through the same message, and
        // they want opposite answers. A label belongs to the page and is painted
        // in the window's own colour; a field is a box on the page and is painted
        // on `COLOR_WINDOW`, which is what makes it read as somewhere to type.
        //
        // The two are told apart by their window class, which is the only thing
        // that distinguishes them: Windows sends this message for a static, for a
        // read-only edit - which is what the hotkey field is - and for a disabled
        // one. An edit is the only one of those that should come out white.
        WM_CTLCOLORSTATIC => {
            let window_brush = with_state(|state| state.window_brush).unwrap_or_default();
            let control = HWND(lparam.0 as *mut core::ffi::c_void);
            let dc = HDC(wparam.0 as *mut core::ffi::c_void);

            if is_edit(control) {
                // A field is a box on the page: white behind its text, which is
                // what tells somebody there is somewhere to type.
                if !window_brush.is_invalid() {
                    return LRESULT(window_brush.0 as isize);
                }
                return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            }

            // A label paints **no background at all**, and that is the whole point.
            // Handing it a brush — even one of exactly the right colour — makes it
            // fill its entire rectangle, and a label's rectangle is wider than its
            // text. The fill lands on whatever lies under the parts the text does
            // not cover, and the widest thing there is the frame's own border. That
            // is what put a clean band of the page's colour straight through the
            // bottom edge of every group.
            //
            // A null brush is how a control is told to leave its background alone.
            // The text is put in transparent mode as well so it does not fill its
            // own glyph boxes either, and then it simply sits on what is there.
            unsafe {
                let _ = SetBkMode(dc, TRANSPARENT);
            }
            let nothing = unsafe { GetStockObject(NULL_BRUSH) };
            LRESULT(nothing.0 as isize)
        }

        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Whether a control is an edit field.
///
/// `WM_CTLCOLORSTATIC` arrives for a static, for a read-only edit and for a
/// disabled one, and there is nothing in the message that says which. The class
/// name does.
fn is_edit(control: HWND) -> bool {
    let mut name = [0u16; 16];
    let written = unsafe { GetClassNameW(control, &mut name) };
    if written <= 0 {
        return false;
    }
    let name = &name[..written as usize];
    // "Edit", case-insensitively, without allocating.
    name.len() == 4
        && (name[0] | 0x20) == u16::from(b'e')
        && (name[1] | 0x20) == u16::from(b'd')
        && (name[2] | 0x20) == u16::from(b'i')
        && (name[3] | 0x20) == u16::from(b't')
}

/// The window size that gives a client area of this size, given a frame this thick.
///
/// The frame is measured from the window rather than computed: the two dimensions
/// are independent, and this is only the arithmetic of adding one to the other.
fn window_size_for_client(
    client_width: i32,
    client_height: i32,
    frame_width: i32,
    frame_height: i32,
) -> (i32, i32) {
    (client_width + frame_width, client_height + frame_height)
}

/// Resize the window so its client area is exactly what the layout was built for.
///
/// The layout is written in client coordinates and every control in it is placed
/// against the client width it declares. If the window's real client area is a few
/// pixels narrower, the last column of every row overflows the frame; if it is
/// wider, the rows stop short of it. Either way the fix is the same and it is not
/// to guess at the frame: ask the window what it has, take the difference, and give
/// it back.
fn match_exact_client(window: HWND) {
    let Some(layout) = with_state(|state| state.layout) else {
        return;
    };
    let mut outer = RECT::default();
    let mut client = RECT::default();
    unsafe {
        if GetWindowRect(window, &mut outer).is_err() || GetClientRect(window, &mut client).is_err() {
            return;
        }
    }
    let frame_width = (outer.right - outer.left) - (client.right - client.left);
    let frame_height = (outer.bottom - outer.top) - (client.bottom - client.top);
    let (wanted_width, wanted_height) =
        window_size_for_client(layout.client_width, layout.client_height, frame_width, frame_height);
    let current_width = outer.right - outer.left;
    let current_height = outer.bottom - outer.top;
    if wanted_width == current_width && wanted_height == current_height {
        return;
    }
    unsafe {
        let _ = SetWindowPos(
            window,
            None,
            0,
            0,
            wanted_width,
            wanted_height,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Report every control's real rectangle and text, on request.
///
/// The layout's arithmetic is checked by tests, but what Windows actually placed
/// is a separate question, and the difference between the two is what a capture
/// shows. Set `LASSO_TRACE_LAYOUT` to have the window print both.
fn trace_layout() {
    if std::env::var_os("LASSO_TRACE_LAYOUT").is_none() {
        return;
    }
    let Some((window, controls, layout)) =
        with_state(|state| (state.window, state.controls, state.layout))
    else {
        return;
    };
    for control in controls.in_order() {
        if control.is_invalid() {
            continue;
        }
        let mut buf = [0u16; 64];
        let n = unsafe { GetClassNameW(control, &mut buf) };
        let class = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(control, &mut rect) }.is_err() {
            continue;
        }
        let mut origin = Default::default();
        if unsafe { GetWindowRect(window, &mut origin) }.is_err() {
            continue;
        }
        let mut text = [0u16; 80];
        let len = unsafe { GetWindowTextW(control, &mut text) };
        let text = String::from_utf16_lossy(&text[..len.max(0) as usize]);
        eprintln!(
            "[layout] {class:8} client ({:4},{:4}) {:4}x{:4}  {text:?}",
            rect.left - origin.left,
            rect.top - origin.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        );
    }
    // What the layout asked for against what the window actually has. The two
    // matching is the property that keeps the last column of a row inside its
    // frame, so it is worth printing when asked.
    let mut client = RECT::default();
    let mut outer = RECT::default();
    if unsafe { GetClientRect(window, &mut client) }.is_ok()
        && unsafe { GetWindowRect(window, &mut outer) }.is_ok()
    {
        eprintln!(
            "[layout] dpi {} window {}x{} client {}x{} | the layout was built for {}x{}",
            layout.dpi,
            outer.right - outer.left,
            outer.bottom - outer.top,
            client.right - client.left,
            client.bottom - client.top,
            layout.client_width,
            layout.client_height,
        );
    }
    eprintln!(
        "[layout] page: client_width {} pad {} frame {}..{} inset {} label_left {} \
         label_right {} field_left {} inner_right {} field_width {}",
        layout.client_width,
        layout.pad,
        layout.frame_left,
        layout.frame_right,
        layout.inset,
        layout.label_left,
        layout.label_left + layout.label_width,
        layout.field_left,
        layout.inner_right(),
        layout.field_width,
    );
}

/// Ask for a folder and put it in the field.
fn browse_for_folder() {
    let Some(field) = with_state(|state| state.controls.output_dir) else {
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
    if let Some(button) = with_state(|state| state.controls.tray_click) {
        let _ = set_text(button, &tray_click_text(choice));
    }
}

/// Collect the controls, refuse an incomplete window, and close with a result.
fn save() {
    let Some(output_dir) =
        with_state(|state| text_of(state.controls.output_dir).trim().to_string())
    else {
        return;
    };

    if output_dir.is_empty() {
        if let Some(hint) = with_state(|state| state.controls.output_hint) {
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
        if let Some(hint) = with_state(|state| state.controls.hotkey_hint) {
            let _ = set_text(hint, HOTKEY_NEEDS_MODIFIER);
        }
        return;
    };

    updated.hotkey = hotkey;
    updated.output_dir = output_dir;
    updated.tray_left_click = tray_left_click;
    updated.clipboard = with_state(|state| is_checked(state.controls.clipboard)).unwrap_or(false);
    updated.cursor = with_state(|state| is_checked(state.controls.cursor)).unwrap_or(false);
    updated.save_file = with_state(|state| is_checked(state.controls.save_file)).unwrap_or(false);

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

/// Move a window back onto the screen, changing as little as possible.
///
/// A window can be widened past the display it is on — 200% is nearly twice the
/// pixels — and Windows will happily place it so that its right edge, and with it
/// the button row, is off the screen. Only the overflow is taken back, so a window
/// the user has positioned keeps that position.
fn clamp_to_screen(window: HWND) {
    unsafe {
        let mut rect = Default::default();
        if GetWindowRect(window, &mut rect).is_err() {
            return;
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let screen_width = GetSystemMetrics(SM_CXSCREEN);
        let screen_height = GetSystemMetrics(SM_CYSCREEN);

        let x = if width >= screen_width {
            0
        } else {
            rect.left.min(screen_width - width).max(0)
        };
        let y = if height >= screen_height {
            0
        } else {
            rect.top.min(screen_height - height).max(0)
        };

        if x != rect.left || y != rect.top {
            let _ = SetWindowPos(window, Some(HWND_TOP), x, y, width, height, SWP_NOZORDER);
        }
    }
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

        let _ = SetWindowPos(window, Some(HWND_TOP), x, y, width, height, SWP_NOZORDER);
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
        SendMessageW(
            control,
            BM_SETCHECK,
            Some(WPARAM(value as usize)),
            Some(LPARAM(0)),
        );
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
        VK_SHIFT,
        VK_CONTROL,
        VK_MENU,
        VK_LWIN,
        VK_RWIN,
        VK_LSHIFT,
        VK_RSHIFT,
        VK_RCONTROL,
        VK_LMENU,
        VK_RMENU,
    ]
    .iter()
    .any(|modifier| modifier.0 == key)
}

/// Encode text as the null-terminated wide string the Win32 calls expect.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
