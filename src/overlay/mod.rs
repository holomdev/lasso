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
//! The system crosshair cursor marks the pointer, and the selection's own edges are
//! drawn with a white line against a black one so an edge can be placed against any
//! content.
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

pub mod annotation;
mod canvas;
mod draw;
mod geometry;
pub mod toolbar;

use std::cell::RefCell;

use anyhow::{Context, Result};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DEFAULT_GUI_FONT, DeleteObject, EndPaint, FillRect, FrameRect,
    GetStockObject, HBRUSH, HDC, HGDIOBJ, InvalidateRect, PAINTSTRUCT, RestoreDC,
    SaveDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VIRTUAL_KEY, VK_CONTROL, VK_DOWN,
    VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HCURSOR,
    HWND_TOPMOST, IDC_CROSS, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE,
    LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SW_SHOW, SWP_SHOWWINDOW, SetCursor,
    SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TranslateMessage, WM_DESTROY,
    WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN,
    WM_SETCURSOR, WM_TIMER, KillTimer, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::capture::Frame;
use annotation::{Annotations, Colour, Frame as SelectionFrame, Point, Shape, Stroke, Tool};
use geometry::{Edges, Zone};
use toolbar::{Button, Metrics, Placement, Region};

/// The timer that closes the palette after the pointer has left it.
///
/// Its own id rather than zero, so it cannot be confused with any other timer this window
/// might grow later.
const PALETTE_TIMER: usize = 1;

/// How long the palette waits after the pointer leaves before it closes.
///
/// Long enough for a hand that went a little past the swatches to come back, short enough
/// that it never sits in the way of the region being adjusted. Below about a fifth of a
/// second it is not felt at all; above half a second it starts to look stuck.
const PALETTE_GRACE_MS: u32 = 260;

/// A region of the capture, in frame pixels.
///
/// The annotations are already drawn onto the captured pixels by the time this is
/// returned, so the region can be cropped and saved directly: there is nothing else to
/// apply afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Which of the strip's ways out the person took.
    pub action: Action,
}

/// What the person asked for when they finished.
///
/// `Save` is what `Enter` and the window closing both mean, so it is the default:
/// somebody who has just chosen a region almost always wants the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Action {
    #[default]
    Save,
    /// Keep the region on the clipboard and do not write a file.
    Copy,
}

/// Which of the strip's action buttons was pressed, if either.
///
/// Kept apart from [`Action`] because the two are read at different moments: this is what
/// happened, and `Action` is what is handed back once the window has closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pressed {
    Save,
    Copy,
    Cancel,
}

/// Smallest accepted selection, in pixels. Anything smaller is treated as a
/// stray click rather than a region.
const MIN_SIDE: i32 = 4;

/// Padding added when invalidating, so the border, the handles and the label are
/// all repainted rather than left as a fringe of the previous frame.
const DIRTY_PAD: i32 = 40;

const WHITE: COLORREF = COLORREF(0x00FF_FFFF);
const BLACK: COLORREF = COLORREF(0x0000_0000);
const LABEL_BG: COLORREF = COLORREF(0x0020_2020);


/// The stroke width a new stroke starts at, in 96-DPI pixels.
const DEFAULT_THICKNESS: u32 = annotation::THICKNESS_STEPS[0];

/// How thick a stroke may be made, so that the key that changes it cannot run away and
/// leave a stroke wider than the selection.


/// Where the strip goes for the current selection.
///
/// `None` before there is a region worth annotating: the strip is there to act on a
/// selection, and showing it before one exists would put buttons over the capture that
/// do nothing yet.
fn strip_for(canvas: &canvas::Canvas, current: Option<RECT>, metrics: &Metrics) -> Option<Placement> {
    let rect = current.filter(|rect| validate(*rect).is_some())?;
    let region = Region {
        x: rect.left,
        y: rect.top,
        width: (rect.right - rect.left) as u32,
        height: (rect.bottom - rect.top) as u32,
    };
    Some(toolbar::place(
        region,
        canvas.width(),
        canvas.height(),
        metrics,
    ))
}

/// The rectangle a placement covers, for invalidating.
fn rect_of(placement: Placement) -> RECT {
    RECT {
        left: placement.x,
        top: placement.y,
        right: placement.x + placement.width as i32,
        bottom: placement.y + placement.height as i32,
    }
}

/// The rectangle a selection covers, as the annotation model wants it.
fn selection_frame(rect: &RECT) -> SelectionFrame {
    SelectionFrame::new(
        rect.left,
        rect.top,
        (rect.right - rect.left).max(0) as u32,
        (rect.bottom - rect.top).max(0) as u32,
    )
}

/// What the active drag is doing.
///
/// `RECT` only implements `PartialEq`, so this type cannot derive `Eq` either.
///
/// `Annotate` carries only the anchor. The stroke being drawn lives in
/// [`State::drawing`], because a `Pencil` holds a `Vec` and this type is `Copy`, which
/// the rest of the input handling relies on — reading it by value from behind the state
/// borrow is what keeps that code readable.
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
    /// Drawing on the selection, in selection-relative coordinates.
    Annotate { anchor: Point },
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
    /// Every brush the strip is drawn with, made once and released together.
    chrome: crate::theme::Chrome,
    drag: Drag,
    /// Last known pointer position, in client coordinates. `None` until the
    /// pointer first moves over the window, which is also when the marker and
    /// the pointer marker appears.
    pointer: Option<(i32, i32)>,
    current: Option<RECT>,
    result: Option<Selection>,

    // --- annotation ---
    /// The tool a drag inside the selection uses. `Move` until another is chosen, so
    /// that dragging inside still adjusts the region.
    tool: Tool,
    colour: Colour,
    thickness: u32,
    /// Everything drawn on the selection, in coordinates relative to it.
    annotations: Annotations,
    /// The stroke under the pointer, complete except for its last point.
    drawing: Option<Stroke>,
    /// Where the strip was put for the current selection, recomputed whenever the
    /// selection or the scale changes.
    strip: Option<Placement>,
    /// The button under the pointer, for the highlight. Also suppressed while drawing,
    /// when the strip is hidden.
    hovered: Option<Button>,
    /// Whether the colour palette is showing.
    ///
    /// Held rather than derived, because it cannot be derived: the palette is opened by the
    /// pointer being on the colour button and kept open by the pointer being inside the
    /// palette, and those two are not the same test. Without this flag, crossing the empty
    /// space where the swatches would be opened a palette out of nothing.
    open_palette: Option<toolbar::PaletteKind>,
    /// Whether the timer that closes the palette after the pointer has left is running.
    ///
    /// Without it the palette closed the instant the pointer was in neither the strip nor the
    /// panel, which punished anybody whose hand went a little past the swatches: there was no
    /// way back except through the colour button. A short grace is what makes "move onto it"
    /// and "move across it" different actions.
    palette_grace: bool,
    /// An action button of the strip that was pressed, which closes the window.
    pressed: Option<Pressed>,
    /// The strip's metrics for this display's scale.
    metrics: Metrics,
}

impl Drop for State {
    fn drop(&mut self) {
        unsafe {
            // The strip's brushes release themselves with the chrome; these three are the
            // selection's own.
            for brush in [self.white, self.black, self.label_bg] {
                let _ = DeleteObject(HGDIOBJ(brush.0));
            }
        }
    }
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Show the overlay for `frame` and return the chosen region.
///
/// `Ok(None)` means the user cancelled.
pub fn select_region(frame: &Frame) -> Result<Option<Composed>> {
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
            chrome: crate::theme::Chrome::new(),
            drag: Drag::Idle,
            pointer: None,
            current: None,
            result: None,
            tool: Tool::default(),
            colour: Colour::default(),
            thickness: DEFAULT_THICKNESS,
            annotations: Annotations::new(),
            drawing: None,
            strip: None,
            hovered: None,
        open_palette: None,
        palette_grace: false,
            pressed: None,
            metrics: toolbar::metrics_for(crate::ui_layout::factor(unsafe {
                windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd)
            })),
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
        // Three outcomes, not two. `GetMessageW` returns -1 on error, and `BOOL(-1)` is
        // **true**, so `.as_bool()` turns an error into "there is a message" and the loop
        // spins for ever on a message that never arrives: the window stays painted, the
        // process stays alive, and nothing sent to it is ever handled. That is what a dead
        // overlay looks like, so the error case is taken separately.
        loop {
            let result = GetMessageW(&mut msg, None, 0, 0);
            if result.0 == 0 {
                break; // WM_QUIT
            }
            if result.0 == -1 {
                eprintln!(
                    "[overlay] GetMessageW failed: {}",
                    windows::Win32::Foundation::GetLastError().0
                );
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // The annotations are re-drawn onto a fresh copy of the capture here rather than read
    // back from the window's buffer.
    //
    // The window's buffer has them, but it also has the dimmed area, the border, the
    // handles and the pointer marker, and none of those belong in the
    // saved image. Re-drawing the strokes is the only way to keep them and leave the rest
    // behind, and it costs one pass over the strokes — which the drawing code already
    // does every frame.
    let composed = STATE.with(|slot| {
        let mut binding = slot.borrow_mut();
        match binding.take() {
            // The annotations are cloned rather than moved: `State` owns brushes and a
            // device context and is dropped here, so a field cannot be taken out of it.
            // One clone per finished capture is nothing next to the capture itself.
            Some(st) => st
                .result
                .map(|selection| (selection, st.annotations.clone())),
            None => None,
        }
    });

    let Some((selection, annotations)) = composed else {
        return Ok(None);
    };

    // A second canvas rather than the window's buffer: this one has the annotations and
    // nothing else, where the window's also carries the dimmed area, the border, the
    // handles — none of which belong in the file.
    let composed = canvas::Canvas::new(&frame.bgra, width, height)?;
    // Drawn onto the whole canvas and cropped afterwards by the caller, which is what decides
    // what is kept: a stroke that runs past the region is cut by the crop, not by refusing to
    // draw it. That is the only clipping left, and it cannot disagree with the selection because
    // it is the selection.
    // The capture first. `Canvas::new` starts empty — the pixels arrive in its `base` layer and
    // are only copied into the back buffer by `compose`, which the window calls while painting
    // and this canvas never did. Without this the strokes were drawn onto black, and the file and
    // the clipboard got black with the marks on top.
    composed.fill_with_capture();
    let origin = (selection.x, selection.y);
    for stroke in annotations.strokes() {
        draw::draw_stroke(composed.dc(), stroke, origin);
    }
    let bgra = composed.composed_bgra();

    Ok(Some(Composed {
        selection,
        bgra,
        width,
        height,
    }))
}

/// A finished selection, with the annotations already drawn onto the pixels.
///
/// `bgra` covers the whole capture, not the region: the caller crops with the same
/// `to_bgra_sub` and `to_rgba_sub` it already uses, so nothing about cropping changes.
pub struct Composed {
    pub selection: Selection,
    pub bgra: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Screen point encoded in a mouse message's `lParam`.
fn point_from(lparam: LPARAM) -> (i32, i32) {
    let raw = lparam.0 as u32;
    let x = (raw & 0xFFFF) as u16 as i16 as i32;
    let y = ((raw >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}

/// Whether a point is inside the current selection.
fn inside_selection(st: &State, point: (i32, i32)) -> bool {
    st.current.is_some_and(|rect| {
        point.0 >= rect.left
            && point.1 >= rect.top
            && point.0 < rect.right
            && point.1 < rect.bottom
    })
}

/// Add a point to a freehand stroke being drawn.
///
/// Consecutive duplicates are dropped: a mouse move that lands on the same pixel says
/// nothing new, and keeping it would grow the stroke without changing what it looks like.
fn append_point(stroke: &mut Stroke, point: Point) {
    if let Shape::Pencil { points } = &mut stroke.shape {
        if points.last() == Some(&point) {
            return;
        }
        points.push(point);
    }
}

/// Whether a stroke is worth keeping.
///
/// A press with no movement on the two-corner tools is a stray click, not a mark: a
/// rectangle with no size draws nothing anyway, and keeping it would put something in
/// the undo stack that cannot be seen.
fn stroke_worth_keeping(stroke: &Stroke) -> bool {
    match &stroke.shape {
        Shape::Pencil { .. } => true,
        _ => {
            let points = match &stroke.shape {
                Shape::Arrow { from, to }
                | Shape::Rectangle { from, to }
                | Shape::Ellipse { from, to } => [*from, *to],
                Shape::Pencil { .. } => return true,
            };
            points[0] != points[1]
        }
    }
}

/// Handle a click on the strip. Returns whether the click was the strip's.
///
/// Returns `true` even when the button did nothing, because a click on the plate must
/// never fall through to the selection: otherwise aiming at a button that does not apply
/// would move the region instead.
/// Begin a stroke at a point on the screen.
///
/// One function for both cases: the press may land inside the region or outside it, and with a
/// drawing tool armed both mean the same thing. The anchor is held relative to the region's
/// origin like every other annotation coordinate, so a stroke may start at a negative offset
/// when the region does not cover the point — which is exactly what drawing over the dimmed area
/// looks like from the inside.
fn start_a_stroke(st: &mut State, rect: &RECT, point: (i32, i32)) {
    let frame = selection_frame(rect);
    let anchor = frame.to_relative(point.0, point.1);
    let shape = match st.tool {
        Tool::Arrow => Shape::Arrow { from: anchor, to: anchor },
        Tool::Rectangle => Shape::Rectangle {
            from: anchor,
            to: anchor,
        },
        Tool::Ellipse => Shape::Ellipse { from: anchor, to: anchor },
        Tool::Pencil => Shape::Pencil {
            points: vec![anchor],
        },
        Tool::Move => unreachable!("Move is not a drawing tool"),
    };
    st.drawing = Some(Stroke::new(shape, st.colour, st.thickness));
    st.drag = Drag::Annotate { anchor };
    st.active_cursor = st.cursors.cross;
}

/// The palette, when it should be showing.
///
/// Showing means there is a strip, no stroke is being drawn, and the pointer is either over
/// the colour button or already inside the palette. That second condition is what lets the
/// pointer travel from the button to a swatch without the panel disappearing under it — the
/// classic way a hover menu becomes unusable.
fn palette_shown(st: &State) -> Option<toolbar::Palette> {
    let strip = st.strip?;
    let kind = st.open_palette?;
    if matches!(st.drag, Drag::Annotate { .. }) {
        return None;
    }
    Some(toolbar::palette_for(
        kind,
        &strip,
        st.canvas.width(),
        st.canvas.height(),
        &st.metrics,
    ))
}

fn strip_clicked(st: &mut State, point: (i32, i32)) -> bool {
    // The palette is asked first: it opens beside the strip, and a press that lands on it is
    // a choice made, not a button pressed.
    if let Some(palette) = palette_shown(st) {
        if let Some(index) = palette.index_at(&st.metrics, point.0, point.1) {
            match palette.kind {
                toolbar::PaletteKind::Colour => st.colour = Colour::ALL[index],
                toolbar::PaletteKind::Thickness => {
                    st.thickness = annotation::THICKNESS_STEPS[index];
                }
            }
            return true;
        }
    }

    let Some(placement) = st.strip else {
        return false;
    };
    let Some(button) = toolbar::hit(&placement, &st.metrics, point.0, point.1) else {
        return false;
    };

    match button {
        Button::Tool(tool) => {
            st.tool = tool;
            // Drawing and adjusting the region want different cursors, and the cursor
            // is also what tells the person which tool is armed.
            st.active_cursor = if tool.draws() { st.cursors.cross } else { st.cursors.size_all };
        }
        Button::Colour => st.colour = st.colour.next(),
        Button::Thickness => {
            // The same list the palette offers, so clicking the button can never land on a
            // width that has no dot in the palette to mark as chosen.
            st.thickness = annotation::next_thickness(st.thickness);
        }
        Button::Undo => {
            st.annotations.undo();
        }
        Button::Redo => {
            st.annotations.redo();
        }
        // The ways out do not close the window here: they record what was pressed, and
        // the caller that already knows how to close it does so carrying the answer.
        Button::Save => st.pressed = Some(Pressed::Save),
        Button::Copy => st.pressed = Some(Pressed::Copy),
        Button::Cancel => st.pressed = Some(Pressed::Cancel),
    }
    true
}

/// Invalidate the union of several rectangles, padded. `None` entries are
/// skipped, and nothing is invalidated when every entry is `None`.
fn invalidate_rects(hwnd: HWND, rects: &[Option<RECT>], pad: i32) {
    let mut acc: Option<RECT> = None;
    for rect in rects.iter().flatten() {
        acc = Some(match acc {
            Some(existing) => geometry::union(existing, *rect),
            None => *rect,
        });
    }
    if let Some(dirty) = acc {
        let padded = geometry::inflated(&dirty, pad);
        unsafe {
            let _ = InvalidateRect(Some(hwnd), Some(&padded), false);
        }
    }
}

/// Invalidate everything that depends on what is drawn: the selection and the strip.
///
/// Called from every place that changes the drawing or the region, instead of listing
/// rectangles at each call site. A person reported the strip losing its icons and the
/// drawing going with it, and the cause was exactly this: a repaint was invalidated over
/// part of the strip, so the plate and the separators were redrawn where the damaged
/// rectangle reached and the glyphs were not — leaving a strip that is present, correct in
/// size, and empty. Listing the rectangles per call site got it right in most places and
/// wrong in others, which is the kind of bug that only shows on the paths nobody exercised.
///
/// `st.strip` is recomputed by the caller before this, so both the old and the new
/// placement are dirtied and a strip that moves leaves nothing behind.
fn relayout(hwnd: HWND, st: &State, previous: &[Option<RECT>]) {
    let mut rects: Vec<Option<RECT>> = previous.to_vec();
    rects.push(st.current);
    rects.push(st.strip.map(rect_of));
    // The palette opens and closes by the pointer moving, with no other state change, so its
    // area is invalidated every time: otherwise the panel would be painted and never cleared,
    // or cleared and never painted.
    // Both palettes, whichever is showing: a palette that has just closed has to be repainted
    // with the capture, and the moment it closes is the moment its kind is forgotten.
    if let Some(strip) = st.strip {
        for kind in [toolbar::PaletteKind::Colour, toolbar::PaletteKind::Thickness] {
            let palette = toolbar::palette_for(
                kind,
                &strip,
                st.canvas.width(),
                st.canvas.height(),
                &st.metrics,
            );
            rects.push(Some(RECT {
                left: palette.x,
                top: palette.y,
                right: palette.x + palette.width as i32,
                bottom: palette.y + palette.height as i32,
            }));
        }
    }
    // The stroke being drawn, wherever it lies. A mark may be made outside the region now, and
    // the region is not where it lives: without this the outside part is written into the back
    // buffer and never presented, so it appears later, when something else happens to repaint
    // that area — which is what made a box look half drawn until Escape.
    if let (Some(stroke), Some(rect)) = (st.drawing.as_ref(), st.current) {
        let (left, top, right, bottom) = stroke.canvas_extent((rect.left, rect.top));
        if right > left && bottom > top {
            rects.push(Some(RECT {
                left,
                top,
                right,
                bottom,
            }));
        }
    }
    invalidate_rects(hwnd, &rects, DIRTY_PAD);
}

fn shifted() -> bool {
    key_down(VK_SHIFT)
}

/// Whether a key is currently held down.
///
/// The high bit of the key state means "down", which is what separates a held modifier
/// from a key that was merely pressed at some point.
fn key_down(key: VIRTUAL_KEY) -> bool {
    (unsafe { GetKeyState(key.0 as i32) } as u16 & 0x8000) != 0
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
            STATE.with(|slot| {
                let mut binding = slot.borrow_mut();
                let Some(st) = binding.as_mut() else {
                    return;
                };
                let previous_selection = st.current;
                let previous_strip = st.strip;
                st.pointer = Some(point);

                // The strip is asked first. A click on it must never become a
                // selection gesture, or aiming at a button would move the region.
                if strip_clicked(st, point) {
                    // Undo, redo, a colour and a tool all change what is drawn or what the
                    // strip shows, so the selection is recomposed rather than merely
                    // re-presented: the drawing lives in the back buffer.
                    relayout(hwnd, st, &[previous_strip.map(rect_of)]);
                    return;
                }

                // What is under the pointer decides what a press means: an edge resizes,
                // inside draws or moves depending on the armed tool, and anywhere else
                // starts a new region.
                let zone = st
                    .current
                    .map(|rect| geometry::zone_of(&rect, point))
                    .unwrap_or(Zone::Outside);

                match zone {
                    Zone::Resize(edges) => {
                        let rect = st.current.expect("a zone implies a selection");
                        st.drag = Drag::Resize { edges, start: rect };
                        st.active_cursor = st.cursors.for_edges(edges);
                    }
                    Zone::Move if st.tool.draws() => {
                        let rect = st.current.expect("a zone implies a selection");
                        start_a_stroke(st, &rect, point);
                    }
                    Zone::Move => {
                        let rect = st.current.expect("a zone implies a selection");
                        st.drag = Drag::Move {
                            grab: (point.0 - rect.left, point.1 - rect.top),
                            start: rect,
                        };
                        st.active_cursor = st.cursors.size_all;
                    }
                    // A drawing tool armed means the press draws, wherever it lands — over the
                    // dimmed area as well. The region is still changed by dragging its handles
                    // or its inside with the move tool, so nothing is lost by giving the press to
                    // the tool.
                    Zone::Outside if st.tool.draws() && st.current.is_some() => {
                        let rect = st.current.expect("a selection is checked above");
                        start_a_stroke(st, &rect, point);
                    }
                    Zone::Outside => {
                        st.drag = Drag::Create { anchor: point };
                        st.current = None;
                        st.strip = None;
                        st.active_cursor = st.cursors.cross;
                    }
                }
                relayout(
                    hwnd,
                    st,
                    &[
                        previous_selection,
                        previous_strip.map(rect_of),
                    ],
                );
            });
            unsafe {
                let _ = SetCapture(hwnd);
            }
            LRESULT(0)
        }

        WM_MOUSEMOVE => {
            let point = point_from(lparam);
            STATE.with(|slot| {
                let mut binding = slot.borrow_mut();
                let Some(st) = binding.as_mut() else {
                    return;
                };

                let previous_pointer = st.pointer;
                let previous_selection = st.current;
                let previous_strip = st.strip;

                st.pointer = Some(point);
                let mut drew = false;
                match st.drag {
                    Drag::Idle => {
                        // The strip is asked before the selection, so that hovering a
                        // button highlights it and the cursor does not claim the pixel
                        // is part of the region.
                        st.hovered = st
                            .strip
                            .as_ref()
                            .and_then(|placement| {
                                toolbar::hit(placement, &st.metrics, point.0, point.1)
                            });
                        // The palette's own rule, kept in one place and tested there: being
                        // inside it holds it open, but only its own button opens it.
                        let over = match st.hovered {
                            Some(Button::Colour) => Some(toolbar::PaletteKind::Colour),
                            Some(Button::Thickness) => Some(toolbar::PaletteKind::Thickness),
                            _ => None,
                        };
                        let inside = st.open_palette.is_some_and(|kind| {
                            st.strip.is_some_and(|placement| {
                                toolbar::palette_for(
                                    kind,
                                    &placement,
                                    st.canvas.width(),
                                    st.canvas.height(),
                                    &st.metrics,
                                )
                                .keeps_open(point.0, point.1)
                            })
                        });
                        let opened = toolbar::palette_opened(st.open_palette, over, inside);
                        if opened.is_some() {
                            st.open_palette = opened;
                            // Back inside: the clock stops, so nothing flickers for a pointer
                            // moving within the panel.
                            if st.palette_grace {
                                unsafe {
                                    let _ = KillTimer(Some(hwnd), PALETTE_TIMER);
                                }
                                st.palette_grace = false;
                            }
                        } else if st.open_palette.is_some() && !st.palette_grace {
                            // Out of the zone with a palette showing: it stays for a moment
                            // rather than vanishing under a hand that has not finished moving.
                            unsafe {
                                let _ = SetTimer(Some(hwnd), PALETTE_TIMER, PALETTE_GRACE_MS, None);
                            }
                            st.palette_grace = true;
                        }

                        st.active_cursor = if st.hovered.is_some() {
                            st.cursors.cross
                        } else if st.tool.draws() && inside_selection(st, point) {
                            // A drawing tool over the region: the crosshair says the
                            // press will draw rather than move.
                            st.cursors.cross
                        } else {
                            match st.current {
                                Some(rect) => st.cursors.for_zone(geometry::zone_of(&rect, point)),
                                None => st.cursors.cross,
                            }
                        };
                    }
                    Drag::Create { anchor } => {
                        st.current = Some(geometry::rectangle(anchor, point));
                        st.strip = strip_for(&st.canvas, st.current, &st.metrics);
                    }
                    Drag::Move { grab, start } => {
                        let moved = geometry::moved(&start, grab, point);
                        // The step, not the journey. `geometry::moved` measures from the rect
                        // the drag started at, and that distance grows with every mouse move;
                        // applying it each time carried the drawing twice as far as the region
                        // on the first event and further on every one after, which is how a
                        // mark disappeared while the region was being dragged over it.
                        let delta = match previous_selection {
                            Some(before) => (moved.left - before.left, moved.top - before.top),
                            None => (0, 0),
                        };
                        st.current = Some(moved);
                        // The drawing marks what is on the screen, so it stays where it is:
                        // a mark put on a face has to stay on the face when the region is
                        // dragged over it. Coordinates are held relative to the region's
                        // origin, so keeping the mark still means shifting it by the opposite
                        // of the origin's own movement.
                        //
                        // It used to be shifted the same way, and since the origin moves too,
                        // every mark travelled twice as far as the region: the drawing flew out
                        // of the frame and was gone.
                        if delta != (0, 0) {
                            st.annotations = st.annotations.rebased(delta);
                        }
                        // Nothing is discarded. The marks stay where they are on the screen,
                        // and one the region no longer covers is simply not in the file — so
                        // dragging the region back over it finds it again, which is what
                        // "only what is inside is saved" means when it is the crop that decides.
                        st.strip = strip_for(&st.canvas, st.current, &st.metrics);
                    }
                    Drag::Resize { edges, start } => {
                        let resized = geometry::resized(&start, edges, point);
                        // Resizing from the top or the left moves the origin as well as the
                        // size, so the drawing has to be shifted by the opposite of that step
                        // before it is clipped — otherwise everything drawn would slide, and by
                        // as much again on every mouse move as it moved here.
                        let delta = match previous_selection {
                            Some(before) => (resized.left - before.left, resized.top - before.top),
                            None => (0, 0),
                        };
                        st.current = Some(resized);
                        if delta != (0, 0) {
                            st.annotations = st.annotations.rebased(delta);
                        }
                        // As with a move: the marks are not thrown away, only left out of the
                        // file while the region does not cover them.
                        st.strip = strip_for(&st.canvas, st.current, &st.metrics);
                    }
                    Drag::Annotate { anchor } => {
                        drew = true;
                        let Some(stroke) = st.drawing.as_mut() else {
                            return;
                        };
                        let Some(rect) = st.current else {
                            return;
                        };
                        let frame = selection_frame(&rect);
                        let point = frame.to_relative(point.0, point.1);
                        match &mut stroke.shape {
                            Shape::Pencil { .. } => append_point(stroke, point),
                            Shape::Arrow { to, .. }
                            | Shape::Rectangle { to, .. }
                            | Shape::Ellipse { to, .. } => *to = point,
                        }
                        let _ = anchor;
                    }
                }

                // The palette belongs to where the strip is: if the strip has moved, the
                // panel would appear beside a button nobody is pointing at, so it closes.
                // Opening again is one hover away.
                if st.strip != previous_strip {
                    st.open_palette = None;
                }

                // A mouse move that lands on the same pixel and leaves the
                // selection alone has nothing to show, unless something is being
                // drawn — in which case the stroke has just changed.
                if !drew && previous_pointer == Some(point) && previous_selection == st.current {
                    return;
                }
                relayout(
                    hwnd,
                    st,
                    &[
                        previous_selection,
                        previous_strip.map(rect_of),
                    ],
                );
            });
            LRESULT(0)
        }

        WM_LBUTTONUP => {
            let point = point_from(lparam);
            let close = STATE.with(|slot| {
                let mut binding = slot.borrow_mut();
                let Some(st) = binding.as_mut() else {
                    return false;
                };
                st.pointer = Some(point);

                // An action button of the strip finishes the window. Pressing it and
                // releasing somewhere else would be a drag away from the button, but the
                // button is not a control and a person aiming at it has already decided,
                // so the press is enough — which is also why this is read here, where the
                // press is known to have happened on the strip.
                if let Some(pressed) = st.pressed.take() {
                    let width = st.canvas.width();
                    let height = st.canvas.height();
                    st.result = match pressed {
                        Pressed::Cancel => None,
                        Pressed::Save | Pressed::Copy => st
                            .current
                            .map(|rect| geometry::intersect(&rect, width, height))
                            .and_then(validate)
                            .map(|mut selection| {
                                selection.action = match pressed {
                                    Pressed::Copy => Action::Copy,
                                    _ => Action::Save,
                                };
                                selection
                            }),
                    };
                    return true;
                }

                // The stroke is finished before the drag is cleared, because the drag
                // is what says a stroke was in progress.
                if let Drag::Annotate { .. } = st.drag {
                    if let Some(stroke) = st.drawing.take() {
                        if stroke_worth_keeping(&stroke) {
                            st.annotations.add(stroke);
                        }
                    }
                }
                st.drag = Drag::Idle;
                st.active_cursor = match st.current {
                    Some(rect) => st.cursors.for_zone(geometry::zone_of(&rect, point)),
                    None => st.cursors.cross,
                };
                st.strip = strip_for(&st.canvas, st.current, &st.metrics);
                // The stroke has just been added or the region adjusted, so both what is
                // drawn and the strip go through the one place that invalidates them.
                relayout(hwnd, st, &[]);
                false
            });
            unsafe {
                let _ = ReleaseCapture();
            }
            if close {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
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
            let control = key_down(VK_CONTROL);

            if control && (key == b'Z' as u16 || key == b'Y' as u16) {
                // `Ctrl+Y` as well as `Ctrl+Z`, because both are in use across Windows
                // and there is no reason to make somebody guess which one this is.
                let redo = key == b'Y' as u16 || shifted();
                STATE.with(|slot| {
                    let mut binding = slot.borrow_mut();
                    let Some(st) = binding.as_mut() else {
                        return;
                    };
                    if redo {
                        st.annotations.redo();
                    } else {
                        st.annotations.undo();
                    }
                    // There is no record of which pixels a removed stroke touched, so the
                    // whole selection is recomposed rather than the stroke's own area.
                    relayout(hwnd, st, &[]);
                });
                return LRESULT(0);
            }

            // Escape gives up the tool rather than the window while one is armed, so that
            // the common case of "I did not mean to draw" does not throw away the region.
            if key == VK_ESCAPE.0 {
                let disarmed = STATE.with(|slot| {
                    let mut binding = slot.borrow_mut();
                    let Some(st) = binding.as_mut() else {
                        return false;
                    };
                    st.drawing = None;
                    if st.tool.draws() {
                        st.tool = Tool::Move;
                        st.active_cursor = st.cursors.size_all;
                        return true;
                    }
                    st.result = None;
                    false
                });
                if disarmed {
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                } else {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                return LRESULT(0);
            }

            if key == VK_RETURN.0 {
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
                return LRESULT(0);
            }

            // The tools, by their first letter. Choosing one does not disturb the region
            // or the drawing: only what the next press means.
            let chosen = match key as u8 as char {
                'S' | 's' => Some(Tool::Move),
                'A' | 'a' => Some(Tool::Arrow),
                'R' | 'r' => Some(Tool::Rectangle),
                'E' | 'e' => Some(Tool::Ellipse),
                'P' | 'p' => Some(Tool::Pencil),
                _ => None,
            };
            if let Some(tool) = chosen {
                STATE.with(|slot| {
                    let mut binding = slot.borrow_mut();
                    let Some(st) = binding.as_mut() else {
                        return;
                    };
                    st.tool = tool;
                    st.drawing = None;
                    st.active_cursor = if tool.draws() {
                        st.cursors.cross
                    } else {
                        st.cursors.size_all
                    };
                    // The strip draws the armed tool, so it has to be redrawn.
                    relayout(hwnd, st, &[]);
                });
                return LRESULT(0);
            }

            if key as u8 as char == 'c' || key as u8 as char == 'C' {
                STATE.with(|slot| {
                    if let Some(st) = slot.borrow_mut().as_mut() {
                        st.colour = st.colour.next();
                        relayout(hwnd, st, &[]);
                    }
                });
                return LRESULT(0);
            }

            if key == b'+' as u16 || key == b'=' as u16 || key == b'-' as u16 {
                STATE.with(|slot| {
                    if let Some(st) = slot.borrow_mut().as_mut() {
                        st.thickness = if key == b'-' as u16 {
                            annotation::previous_thickness(st.thickness)
                        } else {
                            annotation::next_thickness(st.thickness)
                        };
                        relayout(hwnd, st, &[]);
                    }
                });
                return LRESULT(0);
            }

            // The arrows nudge the region only while the region is what is being
            // adjusted: with a drawing tool armed they would move the frame out from
            // under the stroke that is about to be drawn on it.
            if let Some(delta) = arrow_delta(key, if shifted() { 10 } else { 1 }) {
                STATE.with(|slot| {
                    let mut binding = slot.borrow_mut();
                    let Some(st) = binding.as_mut() else {
                        return;
                    };
                    if st.tool.draws() {
                        return;
                    }
                    let Some(nudged) = st.current.map(|rect| translate(&rect, delta)) else {
                        return;
                    };
                    let previous = st.current.replace(nudged);
                    st.annotations = st.annotations.moved(delta.0, delta.1);
                    let previous_strip = st.strip;
                    st.strip = strip_for(&st.canvas, st.current, &st.metrics);
                    relayout(hwnd, st, &[previous, previous_strip.map(rect_of)]);
                });
            }
            LRESULT(0)
        }

        WM_TIMER if wparam.0 == PALETTE_TIMER => {
            STATE.with(|slot| {
                let mut binding = slot.borrow_mut();
                let Some(st) = binding.as_mut() else {
                    return;
                };
                unsafe {
                    let _ = KillTimer(Some(hwnd), PALETTE_TIMER);
                }
                st.palette_grace = false;
                if st.open_palette.is_some() {
                    st.open_palette = None;
                    // Its area has to be repainted with the capture now that it is gone.
                    relayout(hwnd, st, &[]);
                }
            });
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
        action: Action::default(),
    })
}

fn full_screen(canvas: &canvas::Canvas) -> Selection {
    Selection {
        x: 0,
        y: 0,
        width: canvas.width(),
        height: canvas.height(),
        action: Action::default(),
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

        // The drawing goes on after the bright copy of the capture and before the border.
        //
        // It is **not** clipped to the selection. A mark may be made anywhere on the screen — on
        // the dimmed area as well — and the file keeps only what the region covers, which is
        // decided by cropping the capture rather than by refusing to draw. Clipping here is what
        // used to make a stroke stop dead at the region's edge.
        if let Some(rect) = selection {
            let origin = (rect.left, rect.top);
            for stroke in st.annotations.strokes() {
                draw::draw_stroke(mem, stroke, origin);
            }
            if let Some(stroke) = st.drawing.as_ref() {
                draw::draw_stroke(mem, stroke, origin);
            }
        }

        if let Some(rect) = st.current {
            draw_border(mem, &rect, st.white, st.black);
            // The size lives on the strip now, so a separate floating label would say
            // the same thing twice and cover the capture to do it.
            if selection.is_some() && st.strip.is_none() {
                draw_size_label(mem, &rect, st.label_bg);
            }
            // Handles are noise while the region is still being dragged out, and while a
            // drawing tool is armed they would invite a resize that the next press would
            // not do.
            if !matches!(st.drag, Drag::Create { .. }) && !st.tool.draws() {
                draw_handles(mem, &rect, st.white, st.black);
            }
        }

        // The strip is drawn last, so nothing lands on top of it, and it is drawn **every**
        // frame there is one.
        //
        // It used to be skipped while a stroke was being drawn, to save repainting twelve
        // glyphs during a drag. That was a false economy with a visible cost: the repaint
        // clears the strip's area along with everything else, so skipping the drawing left
        // the dimmed desktop there instead, and the strip vanished for as long as the drag
        // lasted and came back when it ended. It looked like a fault, and it was one.
        match st.strip {
            Some(placement) => {
                draw_strip(mem, st, &placement);
                draw_keys_hint(mem, &placement, &st.canvas);
                // The palette goes after the strip, so it is over it where the two meet, and
                // after the hint, so the hint is never drawn on top of a swatch.
                if let Some(palette) = palette_shown(st) {
                    draw::draw_palette(
                        mem,
                        &palette,
                        &st.metrics,
                        &strip_state(st),
                        &st.chrome,
                                    );
                }
            }
            None => draw_hint(mem, &st.canvas),
        }

        // The window before it is presented, so what is written is what the screen shows.
        // Off unless asked for, because a program that writes a file on every repaint is a
        // program that fills a disk.
        dump_canvas(st);

        unsafe {
            let _ = RestoreDC(mem, saved);
        }
    }

    st.canvas.present(window_dc, invalid);
}

/// Write the composed buffer a few times, when asked to.
///
/// A fault that shows as "the drawing went black" cannot be told apart by looking at the
/// window from "the drawing was never composed": both are a dark rectangle where something
/// was. This separates them, and it is the only way to see what a repaint that nobody
/// triggered actually put in the buffer.
fn dump_canvas(st: &State) {
    use std::sync::atomic::{AtomicU32, Ordering};
    /// Enough to catch the first repaint after the fault, few enough to be harmless.
    const MOST: u32 = 40;

    static COUNT: AtomicU32 = AtomicU32::new(0);
    let Ok(dir) = std::env::var("LASSO_DUMP_CANVAS") else {
        return;
    };
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    if n >= MOST {
        return;
    }
    let dir = std::path::PathBuf::from(dir);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join(format!("paint-{n:03}.bmp"));
    if st.canvas.dump_to(&path).is_err() {
        return;
    }
    // Once per paint would be a wall of text; the first, the last and every eighth is
    // enough to see the sequence.
    if n == 0 || n + 1 == MOST || n % 8 == 0 {
        eprintln!(
            "[overlay] canvas dump {n} -> {} ({} GDI objects)",
            path.display(),
            gdi_objects()
        );
    }
}

/// The strip, with the armed tool, the hovered button and the unavailable ones showing.
/// What the strip and its palette are drawn with.
///
/// One builder rather than two: a palette that showed a different colour from the button that
/// opened it would be a defect of exactly the kind this state exists to prevent.
fn strip_state(st: &State) -> draw::StripState {
    draw::StripState {
        // Always one of them: Move is a choice like the others, and the strip starts in it.
        armed: Some(toolbar::Button::Tool(st.tool)),
        hovered: st.hovered,
        pressed: st.pressed.map(|pressed| match pressed {
            Pressed::Save => toolbar::Button::Save,
            Pressed::Copy => toolbar::Button::Copy,
            Pressed::Cancel => toolbar::Button::Cancel,
        }),
        can_undo: st.annotations.can_undo(),
        can_redo: st.annotations.can_redo(),
        colour: st.colour,
        thickness: st.thickness,
    }
}

fn draw_strip(hdc: HDC, st: &State, placement: &Placement) {
    draw::draw_strip(hdc, placement, &st.metrics, &strip_state(st), &st.chrome);
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

/// A line under the strip listing the keys the strip cannot show.
///
/// The buttons carry the tools, so this only has to say what a button cannot: how to put
/// the region back to being movable, and how to reach undo — the two things a person
/// looks for first and would otherwise have to guess.
fn draw_keys_hint(hdc: HDC, placement: &Placement, canvas: &canvas::Canvas) {
    let text = "Esc: move again   -   Ctrl+Z: undo   -   Ctrl+Y: redo   -   C: colour   -   + -: width";
    // Under the strip when there is room, above it otherwise, so it never lands on the
    // selection it is describing.
    let below = placement.y + placement.height as i32 + 4;
    let y = if below + 18 <= canvas.height() as i32 {
        below
    } else {
        (placement.y - 20).max(0)
    };
    draw_text(hdc, placement.x, y, text);
}

fn draw_text(hdc: HDC, x: i32, y: i32, text: &str) {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, WHITE);
        let _ = TextOutW(hdc, x, y, &utf16);
    }
}

/// How many GDI objects this process holds.
///
/// A strip whose plate and separators are drawn and whose glyphs are not is what running
/// out of them looks like, because `FillRect` creates nothing while each glyph needs a pen.
/// Printing this beside every dump turns "the icons went" into a number that either climbs
/// or does not.
pub fn gdi_objects() -> u32 {
    use windows::Win32::System::Threading::{GetCurrentProcess, GetGuiResources, GR_GDIOBJECTS};
    unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) }
}
