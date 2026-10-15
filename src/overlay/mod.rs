//! Interactive region selection overlay.
//!
//! A borderless topmost window covers the whole virtual desktop and shows the
//! frozen capture. Dragging with the left button defines a region: everything
//! outside it is dimmed, the edges get a contrasting border and a live
//! `width x height` label follows the selection.
//!
//! Once a region exists it can be adjusted instead of redrawn: dragging inside
//! moves it, dragging one of the eight handles resizes it, and the arrow keys
//! nudge it one pixel at a time (ten with Shift).
//!
//! Keys and buttons:
//!
//! * left drag on empty space - define a new region
//! * left drag inside         - move the region
//! * left drag on an edge     - resize from that edge or corner
//! * arrows                   - nudge by 1 px, or 10 px with Shift
//! * Enter                    - accept, or the whole screen when nothing is selected
//! * Esc / right click        - cancel
//!
//! The window is created over the exact virtual-desktop rectangle, so client
//! coordinates map 1:1 onto capture pixels and the selected rectangle can be
//! used to crop the frame directly.
//!
//! Painting is double buffered: every layer is composed into the back buffer
//! held by [`canvas::Canvas`] and the result reaches the window with a single
//! blit, so a drag never shows half-drawn frames.

mod canvas;
mod geometry;

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
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN,
    VK_RIGHT, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HCURSOR,
    HWND_TOPMOST, IDC_CROSS, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE,
    LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SW_SHOW, SWP_SHOWWINDOW, SetCursor,
    SetForegroundWindow, SetWindowPos, ShowWindow, TranslateMessage, WM_DESTROY, WM_ERASEBKGND,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN, WM_SETCURSOR,
    WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::capture::Frame;
use geometry::{Edges, Zone};

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

/// Padding added when invalidating, so the border, the handles and the label are
/// all repainted.
const DIRTY_PAD: i32 = 40;

const WHITE: COLORREF = COLORREF(0x00FF_FFFF);
const BLACK: COLORREF = COLORREF(0x0000_0000);
const LABEL_BG: COLORREF = COLORREF(0x0020_2020);

/// What the active drag is doing.
///
/// `RECT` only implements `PartialEq`, so this type cannot derive `Eq` either.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    Idle,
    /// Defining a new region from `anchor`.
    Create { anchor: (i32, i32) },
    /// Moving the whole selection; `grab` is the offset from its origin to the
    /// point that was grabbed.
    Move { grab: (i32, i32), start: RECT },
    /// Resizing; only these sides follow the pointer.
    Resize { edges: Edges, start: RECT },
}

/// The cursors the overlay switches between. All are shared system cursors, so
/// none of them needs destroying.
struct Cursors {
    cross: HCURSOR,
    size_all: HCURSOR,
    size_we: HCURSOR,
    size_ns: HCURSOR,
    size_nwse: HCURSOR,
    size_nesw: HCURSOR,
}

impl Cursors {
    fn load() -> Result<Self> {
        unsafe {
            Ok(Self {
                cross: LoadCursorW(None, IDC_CROSS).context("LoadCursorW(IDC_CROSS) failed")?,
                size_all: LoadCursorW(None, IDC_SIZEALL).context("LoadCursorW(IDC_SIZEALL) failed")?,
                size_we: LoadCursorW(None, IDC_SIZEWE).context("LoadCursorW(IDC_SIZEWE) failed")?,
                size_ns: LoadCursorW(None, IDC_SIZENS).context("LoadCursorW(IDC_SIZENS) failed")?,
                size_nwse: LoadCursorW(None, IDC_SIZENWSE)
                    .context("LoadCursorW(IDC_SIZENWSE) failed")?,
                size_nesw: LoadCursorW(None, IDC_SIZENESW)
                    .context("LoadCursorW(IDC_SIZENESW) failed")?,
            })
        }
    }

    fn for_edges(&self, edges: Edges) -> HCURSOR {
        match edges.diagonal() {
            Some(geometry::Diagonal::NwSe) => self.size_nwse,
            Some(geometry::Diagonal::NeSw) => self.size_nesw,
            None => {
                if edges.left || edges.right {
                    self.size_we
                } else {
                    self.size_ns
                }
            }
        }
    }

    fn for_zone(&self, zone: Zone) -> HCURSOR {
        match zone {
            Zone::Resize(edges) => self.for_edges(edges),
            Zone::Move => self.size_all,
            Zone::Outside => self.cross,
        }
    }
}

struct State {
    canvas: canvas::Canvas,
    cursors: Cursors,
    active_cursor: HCURSOR,
    white: HBRUSH,
    black: HBRUSH,
    label_bg: HBRUSH,
    drag: Drag,
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
    let cursors = Cursors::load()?;

    let hmodule = unsafe { GetModuleHandleW(None) }.context("GetModuleHandleW failed")?;
    let hinstance = HINSTANCE(hmodule.0);
    let class_name = w!("LassoOverlay");

    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance,
        hCursor: cursors.cross,
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
            active_cursor: cursors.cross,
            cursors,
            white: unsafe { CreateSolidBrush(WHITE) },
            black: unsafe { CreateSolidBrush(BLACK) },
            label_bg: unsafe { CreateSolidBrush(LABEL_BG) },
            drag: Drag::Idle,
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

/// Repaint only the area the selection touched, plus padding for the border,
/// the handles and the label. `None` rectangles are simply skipped.
fn invalidate_around(hwnd: HWND, previous: Option<RECT>, current: Option<RECT>) {
    let dirty = match (previous, current) {
        (Some(p), Some(c)) => geometry::union(p, c),
        (Some(p), None) => p,
        (None, Some(c)) => c,
        (None, None) => return,
    };
    let padded = geometry::inflated(&dirty, DIRTY_PAD);
    unsafe {
        let _ = InvalidateRect(Some(hwnd), Some(&padded), false);
    }
}

fn shifted() -> bool {
    // The high bit of the key state means "currently down".
    (unsafe { GetKeyState(VK_SHIFT.0 as i32) } as u16 & 0x8000) != 0
}

/// Pixel delta for an arrow key, larger while Shift is held.
fn arrow_delta(key: u16, step: i32) -> Option<(i32, i32)> {
    if key == VK_LEFT.0 {
        Some((-step, 0))
    } else if key == VK_RIGHT.0 {
        Some((step, 0))
    } else if key == VK_UP.0 {
        Some((0, -step))
    } else if key == VK_DOWN.0 {
        Some((0, step))
    } else {
        None
    }
}

fn translate(rect: &RECT, delta: (i32, i32)) -> RECT {
    RECT {
        left: rect.left + delta.0,
        top: rect.top + delta.1,
        right: rect.right + delta.0,
        bottom: rect.bottom + delta.1,
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // Handled so Windows never paints a background behind the capture; an
        // erased background is the other classic source of flicker.
        WM_ERASEBKGND => LRESULT(1),

        WM_SETCURSOR => {
            STATE.with(|slot| {
                if let Some(st) = slot.borrow().as_ref() {
                    unsafe {
                        SetCursor(Some(st.active_cursor));
                    }
                }
            });
            LRESULT(1)
        }

        WM_LBUTTONDOWN => {
            let point = point_from(lparam);
            let (previous, current) = STATE.with(|slot| {
                let mut binding = slot.borrow_mut();
                let Some(st) = binding.as_mut() else {
                    return (None, None);
                };
                let previous = st.current;
                match st.current.map(|rect| (rect, geometry::zone_of(&rect, point))) {
                    Some((rect, Zone::Resize(edges))) => {
                        st.drag = Drag::Resize { edges, start: rect };
                        st.active_cursor = st.cursors.for_edges(edges);
                    }
                    Some((rect, Zone::Move)) => {
                        st.drag = Drag::Move {
                            grab: (point.0 - rect.left, point.1 - rect.top),
                            start: rect,
                        };
                        st.active_cursor = st.cursors.size_all;
                    }
                    _ => {
                        st.drag = Drag::Create { anchor: point };
                        st.current = None;
                        st.active_cursor = st.cursors.cross;
                    }
                }
                (previous, st.current)
            });
            unsafe {
                let _ = SetCapture(hwnd);
            }
            // Starting away from the selection drops it, so the previous region
            // has to be dimmed back immediately.
            invalidate_around(hwnd, previous, current);
            LRESULT(0)
        }

        WM_MOUSEMOVE => {
            let point = point_from(lparam);
            let mut dirty = None;
            STATE.with(|slot| {
                let mut binding = slot.borrow_mut();
                let Some(st) = binding.as_mut() else {
                    return;
                };
                let next = match st.drag {
                    Drag::Idle => {
                        // Keep the cursor in step with the zone under the
                        // pointer, which is what makes the handles discoverable.
                        st.active_cursor = match st.current {
                            Some(rect) => st.cursors.for_zone(geometry::zone_of(&rect, point)),
                            None => st.cursors.cross,
                        };
                        None
                    }
                    Drag::Create { anchor } => Some(geometry::rectangle(anchor, point)),
                    Drag::Move { grab, start } => Some(geometry::moved(&start, grab, point)),
                    Drag::Resize { edges, start } => Some(geometry::resized(&start, edges, point)),
                };
                if let Some(next) = next {
                    // Sub-pixel mouse movement lands on the same rectangle;
                    // repainting it again would only add work and flicker.
                    if st.current != Some(next) {
                        let previous = st.current.replace(next);
                        dirty = Some((previous, Some(next)));
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
            let (previous, current) = STATE.with(|slot| {
                let mut binding = slot.borrow_mut();
                let Some(st) = binding.as_mut() else {
                    return (None, None);
                };
                st.drag = Drag::Idle;
                st.active_cursor = match st.current {
                    Some(rect) => st.cursors.for_zone(geometry::zone_of(&rect, point)),
                    None => st.cursors.cross,
                };
                (st.current, st.current)
            });
            unsafe {
                let _ = ReleaseCapture();
            }
            invalidate_around(hwnd, previous, current);
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
                    let mut binding = slot.borrow_mut();
                    let Some(st) = binding.as_mut() else {
                        return false;
                    };
                    let width = st.canvas.width();
                    let height = st.canvas.height();
                    // A region nudged or moved past the edge is clamped rather
                    // than rejected.
                    st.result = st
                        .current
                        .map(|rect| geometry::intersect(&rect, width, height))
                        .and_then(validate)
                        .or_else(|| Some(full_screen(&st.canvas)));
                    true
                });
                if close {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            } else if let Some(delta) = arrow_delta(key, if shifted() { 10 } else { 1 }) {
                let mut dirty = None;
                STATE.with(|slot| {
                    if let Some(st) = slot.borrow_mut().as_mut() {
                        if let Some(nudged) = st.current.map(|rect| translate(&rect, delta)) {
                            let previous = st.current.replace(nudged);
                            dirty = Some((previous, Some(nudged)));
                        }
                    }
                });
                if let Some((previous, current)) = dirty {
                    invalidate_around(hwnd, previous, current);
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
                // Handles are noise while the region is still being dragged out.
                if !matches!(st.drag, Drag::Create { .. }) {
                    draw_handles(mem, &rect, st.white, st.black);
                }
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
    let outer = geometry::inflated(rect, 1);
    unsafe {
        FrameRect(hdc, &outer, black);
        FrameRect(hdc, rect, white);
    }
}

/// The eight resize grips: four corners and four edge midpoints.
fn draw_handles(hdc: HDC, rect: &RECT, white: HBRUSH, black: HBRUSH) {
    let mid_x = (rect.left + rect.right) / 2;
    let mid_y = (rect.top + rect.bottom) / 2;
    let grips = [
        (rect.left, rect.top),
        (mid_x, rect.top),
        (rect.right, rect.top),
        (rect.left, mid_y),
        (rect.right, mid_y),
        (rect.left, rect.bottom),
        (mid_x, rect.bottom),
        (rect.right, rect.bottom),
    ];

    let half = geometry::HANDLE / 2;
    for (x, y) in grips {
        let square = RECT {
            left: x - half,
            top: y - half,
            right: x - half + geometry::HANDLE,
            bottom: y - half + geometry::HANDLE,
        };
        unsafe {
            FillRect(hdc, &square, white);
            FrameRect(hdc, &square, black);
        }
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
    let text = "Drag to select   -   inside: move   -   edges: resize   -   arrows: nudge   -   Enter: save   -   Esc: cancel";
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
