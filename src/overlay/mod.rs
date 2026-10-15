//! Interactive region selection overlay.
//!
//! A borderless topmost window covers the whole virtual desktop and shows the
//! frozen capture. Dragging with the left button defines a region: everything
//! outside it is dimmed, the edges get a contrasting border and a live
//! `width x height` label follows the selection.
//!
//! Keys and buttons:
//!
//! * left drag   - define the region
//! * Enter       - accept the region, or the whole screen when none is selected
//! * Esc         - cancel
//! * right click - cancel
//!
//! The window is created over the exact virtual-desktop rectangle, so client
//! coordinates map 1:1 onto capture pixels and the selected rectangle can be
//! used to crop the frame directly.
//!
//! Painting is double buffered: every layer is composed into the back buffer
//! held by [`canvas::Canvas`] and the result reaches the window with a single
//! blit, so a drag never shows half-drawn frames.

mod canvas;

use std::cell::RefCell;

use anyhow::{Context, Result};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DEFAULT_GUI_FONT, DeleteObject, EndPaint, FillRect, FrameRect,
    GetStockObject, HBRUSH, HDC, HGDIOBJ, InvalidateRect, PAINTSTRUCT, RestoreDC, SaveDC,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HCURSOR,
    HWND_TOPMOST, IDC_CROSS, LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SW_SHOW,
    SWP_SHOWWINDOW, SetCursor, SetForegroundWindow, SetWindowPos, ShowWindow, TranslateMessage,
    WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_RBUTTONDOWN, WM_SETCURSOR, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::capture::Frame;

/// A region of the capture, in frame pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Smallest accepted selection, in pixels. Anything smaller is treated as a
/// stray click rather than a region.
const MIN_SIDE: i32 = 4;

/// Padding added when invalidating, so the border and the label are repainted.
const DIRTY_PAD: i32 = 40;

const WHITE: COLORREF = COLORREF(0x00FF_FFFF);
const BLACK: COLORREF = COLORREF(0x0000_0000);
const LABEL_BG: COLORREF = COLORREF(0x0020_2020);

struct State {
    canvas: canvas::Canvas,
    cursor: HCURSOR,
    white: HBRUSH,
    black: HBRUSH,
    label_bg: HBRUSH,
    dragging: bool,
    anchor: (i32, i32),
    current: Option<RECT>,
    result: Option<Selection>,
}

impl Drop for State {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.white.0));
            let _ = DeleteObject(HGDIOBJ(self.black.0));
            let _ = DeleteObject(HGDIOBJ(self.label_bg.0));
        }
    }
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Show the overlay for `frame` and return the chosen region.
///
/// `Ok(None)` means the user cancelled.
pub fn select_region(frame: &Frame) -> Result<Option<Selection>> {
    let width = frame.width;
    let height = frame.height;
    let (vx, vy) = frame.origin;

    // Built before the window so a failure here cannot leave a window behind.
    let canvas = canvas::Canvas::new(&frame.bgra, width, height)?;

    let hmodule = unsafe { GetModuleHandleW(None) }.context("GetModuleHandleW failed")?;
    let hinstance = HINSTANCE(hmodule.0);
    let class_name = w!("LassoOverlay");

    let cursor = unsafe { LoadCursorW(None, IDC_CROSS) }.context("LoadCursorW(IDC_CROSS) failed")?;

    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance,
        hCursor: cursor,
        lpszClassName: class_name,
        ..Default::default()
    };
    // A zero return means this process already registered the class, which is
    // fine and expected on a second invocation.
    unsafe {
        RegisterClassW(&wc);
    }

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name,
            w!("lasso"),
            WS_POPUP,
            vx,
            vy,
            width as i32,
            height as i32,
            None,
            None,
            Some(hinstance),
            None,
        )
    }
    .context("CreateWindowExW failed")?;

    STATE.with(|slot| {
        *slot.borrow_mut() = Some(State {
            canvas,
            cursor,
            white: unsafe { CreateSolidBrush(WHITE) },
            black: unsafe { CreateSolidBrush(BLACK) },
            label_bg: unsafe { CreateSolidBrush(LABEL_BG) },
            dragging: false,
            anchor: (0, 0),
            current: None,
            result: None,
        });
    });

    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            vx,
            vy,
            width as i32,
            height as i32,
            SWP_SHOWWINDOW,
        );
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    let selection = STATE.with(|slot| slot.borrow_mut().take().and_then(|st| st.result));
    Ok(selection)
}

/// Screen point encoded in a mouse message's `lParam`.
fn point_from(lparam: LPARAM) -> (i32, i32) {
    let raw = lparam.0 as u32;
    let x = (raw & 0xFFFF) as u16 as i16 as i32;
    let y = ((raw >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}

/// Rectangle spanned by two points, with positive extents.
fn rect_from(a: (i32, i32), b: (i32, i32)) -> RECT {
    RECT {
        left: a.0.min(b.0),
        top: a.1.min(b.1),
        right: a.0.max(b.0),
        bottom: a.1.max(b.1),
    }
}

fn union(a: RECT, b: RECT) -> RECT {
    RECT {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    }
}

fn inflate(r: &RECT, by: i32) -> RECT {
    RECT {
        left: r.left - by,
        top: r.top - by,
        right: r.right + by,
        bottom: r.bottom + by,
    }
}

/// Repaint only the area the selection touched, plus padding for the border and
/// the label. `None` rectangles are simply skipped.
fn invalidate_around(hwnd: HWND, previous: Option<RECT>, current: Option<RECT>) {
    let dirty = match (previous, current) {
        (Some(p), Some(c)) => union(p, c),
        (Some(p), None) => p,
        (None, Some(c)) => c,
        (None, None) => return,
    };
    let padded = inflate(&dirty, DIRTY_PAD);
    unsafe {
        let _ = InvalidateRect(Some(hwnd), Some(&padded), false);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // Handled so Windows never paints a background behind the capture;
        // an erased background is the other classic source of flicker.
        WM_ERASEBKGND => LRESULT(1),

        WM_SETCURSOR => {
            STATE.with(|slot| {
                if let Some(st) = slot.borrow().as_ref() {
                    unsafe {
                        SetCursor(Some(st.cursor));
                    }
                }
            });
            LRESULT(1)
        }

        WM_LBUTTONDOWN => {
            let point = point_from(lparam);
            let previous = STATE.with(|slot| {
                if let Some(st) = slot.borrow_mut().as_mut() {
                    st.dragging = true;
                    st.anchor = point;
                    st.current.take()
                } else {
                    None
                }
            });
            unsafe {
                let _ = SetCapture(hwnd);
            }
            // A fresh drag starts from nothing, so the previous region has to be
            // dimmed back immediately.
            invalidate_around(hwnd, previous, None);
            LRESULT(0)
        }

        WM_MOUSEMOVE => {
            let point = point_from(lparam);
            let mut dirty = None;
            STATE.with(|slot| {
                if let Some(st) = slot.borrow_mut().as_mut() {
                    if st.dragging {
                        let next = rect_from(st.anchor, point);
                        // Sub-pixel mouse movement lands on the same rectangle;
                        // repainting it again would only add work and flicker.
                        if st.current != Some(next) {
                            let previous = st.current.replace(next);
                            dirty = Some((previous, Some(next)));
                        }
                    }
                }
            });
            if let Some((previous, current)) = dirty {
                invalidate_around(hwnd, previous, current);
            }
            LRESULT(0)
        }

        WM_LBUTTONUP => {
            let point = point_from(lparam);
            let current = STATE.with(|slot| {
                if let Some(st) = slot.borrow_mut().as_mut() {
                    st.dragging = false;
                    st.anchor = point;
                    st.current
                } else {
                    None
                }
            });
            unsafe {
                let _ = ReleaseCapture();
            }
            invalidate_around(hwnd, current, current);
            LRESULT(0)
        }

        WM_RBUTTONDOWN => {
            STATE.with(|slot| {
                if let Some(st) = slot.borrow_mut().as_mut() {
                    st.result = None;
                }
            });
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }

        WM_KEYDOWN => {
            let key = wparam.0 as u16;
            if key == VK_ESCAPE.0 {
                STATE.with(|slot| {
                    if let Some(st) = slot.borrow_mut().as_mut() {
                        st.result = None;
                    }
                });
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            } else if key == VK_RETURN.0 {
                let close = STATE.with(|slot| {
                    if let Some(st) = slot.borrow_mut().as_mut() {
                        st.result = st
                            .current
                            .and_then(validate)
                            .or_else(|| Some(full_screen(&st.canvas)));
                        true
                    } else {
                        false
                    }
                });
                if close {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
            LRESULT(0)
        }

        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            STATE.with(|slot| {
                if let Some(st) = slot.borrow().as_ref() {
                    paint(hdc, st, &ps.rcPaint);
                }
            });
            unsafe {
                let _ = EndPaint(hwnd, &ps);
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

fn validate(rect: RECT) -> Option<Selection> {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width < MIN_SIDE || height < MIN_SIDE {
        return None;
    }
    Some(Selection {
        x: rect.left,
        y: rect.top,
        width: width as u32,
        height: height as u32,
    })
}

fn full_screen(canvas: &canvas::Canvas) -> Selection {
    Selection {
        x: 0,
        y: 0,
        width: canvas.width(),
        height: canvas.height(),
    }
}

/// Compose every layer into the back buffer and present the invalid region with
/// a single blit.
fn paint(window_dc: HDC, st: &State, invalid: &RECT) {
    let selection = st.current.filter(|rect| validate(*rect).is_some());
    st.canvas.compose(invalid, selection.as_ref());

    let mem = st.canvas.dc();
    let saved = unsafe { SaveDC(mem) };
    if saved != 0 {
        let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
        unsafe {
            SelectObject(mem, font);
        }

        if let Some(rect) = st.current {
            draw_border(mem, &rect, st.white, st.black);
            if selection.is_some() {
                draw_size_label(mem, &rect, st.label_bg);
            }
        }
        draw_hint(mem, &st.canvas);

        unsafe {
            let _ = RestoreDC(mem, saved);
        }
    }

    st.canvas.present(window_dc, invalid);
}

fn draw_border(hdc: HDC, rect: &RECT, white: HBRUSH, black: HBRUSH) {
    let outer = inflate(rect, 1);
    unsafe {
        FrameRect(hdc, &outer, black);
        FrameRect(hdc, rect, white);
    }
}

/// Draw `width x height` next to the selection, on a dark plate so it stays
/// readable over any content.
fn draw_size_label(hdc: HDC, rect: &RECT, plate: HBRUSH) {
    let text = format!("{} x {}", rect.right - rect.left, rect.bottom - rect.top);
    let plate_width = text.chars().count() as i32 * 8 + 12;
    let plate_height = 22;

    // Prefer above the selection; when the selection is glued to the top of the
    // screen, sit it inside the top edge instead.
    let top = if rect.top >= plate_height + 4 {
        rect.top - plate_height - 4
    } else {
        rect.top
    };
    let left = rect.left.max(0);

    let plate_rect = RECT {
        left,
        top,
        right: left + plate_width,
        bottom: top + plate_height,
    };
    unsafe {
        FillRect(hdc, &plate_rect, plate);
    }
    draw_text(hdc, left + 6, top + 4, &text);
}

fn draw_hint(hdc: HDC, canvas: &canvas::Canvas) {
    let text = "Drag to select   -   Enter: save   -   Esc: cancel";
    let width = text.chars().count() as i32 * 7 + 12;
    let x = ((canvas.width() as i32 - width) / 2).max(0);
    let y = (canvas.height() as i32 - 40).max(0);
    draw_text(hdc, x, y, text);
}

fn draw_text(hdc: HDC, x: i32, y: i32, text: &str) {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, WHITE);
        let _ = TextOutW(hdc, x, y, &utf16);
    }
}
