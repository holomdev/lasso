# A Windows desktop application that looks native

A checklist for a native Windows application, independent of the toolkit it is
written in. Everything here was learned the hard way while building `lasso`, and
every item is there because leaving it out produced a visible defect — not because
it is good practice in the abstract.

The order matters. The first section is what to settle before a single window is
drawn, because none of it can be fixed from inside a running program.

---

## 1. The manifest — do this before anything else

Windows reads a manifest to decide two things about a process, both **before `main`
runs**. Neither can be changed afterwards, and both are invisible in a screenshot of
a program that got them wrong.

### 1.1 The common controls, version 6

```xml
<dependency>
  <dependentAssembly>
    <assemblyIdentity
      type="win32"
      name="Microsoft.Windows.Common-Controls"
      version="6.0.0.0"
      processorArchitecture="*"
      publicKeyToken="6595b64144ccf1df"
      language="*"/>
  </dependentAssembly>
</dependency>
```

Without it the loader hands the process the **version 5** controls, and every control
is drawn in the style of Windows 2000. This is not cosmetic:

- The classic group box fills the gap it makes for its own caption in
  `COLOR_WINDOW` — white — inside its own painting code. **No message from your
  window can change it.** `WM_CTLCOLORSTATIC` and `WM_CTLCOLORBTN` both have no
  effect on it, which was measured, not assumed.
- The classic check box, radio button and scroll bar are all the pre-XP design.

### 1.2 Which systems the program knows about

```xml
<compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
  <application>
    <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/> <!-- Windows 10 and 11 -->
    <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/> <!-- Windows 8.1 -->
    <supportedOS Id="{4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38}"/> <!-- Windows 8 -->
    <supportedOS Id="{35138b9a-5d96-4fbd-8e2d-a2440225f93a}"/> <!-- Windows 7 -->
  </application>
</compatibility>
```

Without the list, Windows guesses from the version stamp in the executable, and the
guess brings compatibility shims with it.

### 1.3 DPI awareness

```xml
<application xmlns="urn:schemas-microsoft-com:asm.v3">
  <windowsSettings>
    <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
  </windowsSettings>
</application>
```

Both elements are wanted: the second is what Windows 10 reads, the first is what
newer versions prefer.

### 1.4 Attaching it

`rc.exe` does not accept a `.manifest` directly. It takes a resource script:

```
1 24 "app.manifest"
```

`1` is the manifest for the program itself; `24` is `RT_MANIFEST`. Compile that into
an object and link it:

```
rc.exe /nologo /fo app.res app.rc
# then: cargo:rustc-link-arg-bins=<path to app.res>
```

Three details that cost time:

- **`rc.exe` is not on `PATH`.** It lives under
  `%ProgramFiles(x86)%\Windows Kits\10\bin\<sdk version>\<arch>\rc.exe`.
- **It refuses to build for a different architecture.** Pick the directory matching
  the target, not the host.
- **A machine without the SDK must still build.** Emit a warning that names what the
  executable will be missing, and carry on. A program with no manifest runs; it just
  looks like Windows 2000 and gets scaled by the system.

---

## 2. DPI

### 2.1 Declare it before anything exists

Whatever the manifest says, ask as well, from the first statement of the program,
**before any window or device context is created**:

```
SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
```

Check the result. It fails once a window or a device context exists, and the failure
is the only notice you get that the process is aware for a different mode than you
think. On a fresh Windows install this call is also what stops the system from
bitmap-scaling the whole application.

### 2.2 The layout is arithmetic, and it is testable without a window

Keep every position and size in one module as **pure functions of the scale**, with
no window, no device context and no `GetSystemMetrics` inside it:

- A base size per measurement, in 96-DPI units.
- One `scale(value, factor)` used for all of them, rounding **half up**. Round-half
  to even produces off-by-one gaps that are visible on a row of straight lines.
- Every derived rectangle a function of those, so a change of scale changes all of
  them together.

Then the layout can be unit-tested at 100%, 125%, 150%, 200% and 300% with no
display involved. That is what makes the invariants checkable:

- Nothing overlaps anything else.
- Every control ends **above** the bottom of the container it belongs to, with room
  for that container's own edge. This one was missing, and its absence let a hint
  paint across the bottom of its own frame.
- A label is level with the field beside it: **the same top and the same height**,
  not centred on it. Sizing a label to its own text cannot line it up when the two
  heights have different parity, and the half pixel it is out by shows.

### 2.3 A window's client area is measured, not computed

`AdjustWindowRectEx` is scale-dependent: it reads the thread's awareness, so a
per-monitor-aware process gets its frame scaled a second time when the client size
handed to it was already scaled by the layout. The window then has a client area a
few pixels different from the one the layout was written for, and the last column of
every row overruns its frame.

`AdjustWindowRectExForDpi` is the right call, and **it is still not enough**: it
returned a window whose client area was six pixels off at 150%.

Do this instead, after creating the window:

1. `GetWindowRect` and `GetClientRect`.
2. Take the difference — that is the frame, whatever it is made of.
3. `SetWindowPos` with the client size the layout declares, plus that difference.
4. Assert in a test that the client and the layout agree.

At 100% the correction is zero, so it costs one call.

### 2.4 The font

Do not use the stock GUI font. Create one with `CreateFontW`, taking its face name
and weight from `GetStockObject(DEFAULT_GUI_FONT)` through `GetObjectW`, and its
height scaled by the same factor as everything else. A window whose text does not
grow with its controls looks broken at every scale above 100%.

If a control's width depends on its text, measure it with `GetTextExtentPoint32W` in
that font, and keep the function that does it injectable or callable in a test.

### 2.5 Handle a scale change

On `WM_DPICHANGED`, a control cannot be resized to a *different font*; it has to be
replaced. Destroy the children, rebuild them at the new scale, and restore their
contents:

- Read every value the user could have changed **before** destroying anything.
- Hide the window, destroy, clear the background, rebuild, show again, restore.
- Do this from the state, never from a nested `with_state`-style borrow, which
  silently returns nothing when it re-enters.

### 2.6 The window must fit the screen

A layout written for one scale is taller on a bigger one. Clamp the finished window
to the work area of the monitor it opened on, and test it at 300%.

---

## 3. Colours

### 3.1 Take them from the system

`GetSysColor(COLOR_BTNFACE)`, `COLOR_WINDOW`, `COLOR_BTNTEXT`, and the rest. Do not
write literals: a literal is wrong on a system with a different theme, and it is
wrong in high-contrast mode by definition.

Create solid brushes once, own them in the window's state, and release them on every
path out — including the failure paths, which is what a single teardown function is
for.

### 3.2 The window class background

`hbrBackground: HBRUSH(COLOR_BTNFACE + 1)` — the value plus one is how a class names a
system colour. In Rust with the `windows` crate the constant is a newtype, so it is
`COLOR_BTNFACE.0 as usize + 1`.

### 3.3 What a control asks for its background

This is the part that produces the odd-looking results, because **the same message
arrives for controls that want opposite answers**:

| control | message | what it wants |
|---|---|---|
| static text, and a **read-only edit** | `WM_CTLCOLORSTATIC` | a field wants a white brush; a label wants **no brush at all** |
| button, and a group box | `WM_CTLCOLORBTN` | the page's colour |
| a control you draw yourself | `WM_DRAWITEM` | you paint it |

The message does **not** say which control it is about. The window class does, so
ask `GetClassNameW` and compare it case-insensitively, without allocating.

**A label must be given no brush.** A static handed a brush fills its *entire
rectangle*, and a label's rectangle is wider than its text: the fill lands on
whatever lies under the parts the words do not cover. For a full-width hint that is
the frame's own border, so the frame gets a clean band of the wrong colour across
it. Return `GetStockObject(NULL_BRUSH)` and set the text mode to transparent:
nothing is painted and the words sit on what is already there.

**A group box must be given a brush**, for the opposite reason: it erases a gap in
its own line for its caption, and with no brush the line runs through the words.

### 3.4 A theme paints what it likes

A themed control may ignore your answer entirely. Measured: the group box's caption
background is white whether `WM_CTLCOLORBTN` is handled or falls through. Proving
this needs a deliberately wrong colour, not a right one — return magenta, capture,
and see **which pixels** it reaches. A right colour that is not used looks exactly
like a right colour that is.

---

## 4. Layout: what goes wrong

- **Derive every container's height from its contents**, never from a constant. A
  group two pixels shorter than its own contents lets the last control paint over the
  frame. Add the test that asserts otherwise.
- **Keep the bottom edge clear.** A container's height has to include room for its
  own frame, not just stop at the last control.
- **A hint must not reach the container's inner edge.** A full-width static's
  background lands on the border even when its text does not.
- **One uniform grid.** Left margin, label column, gap, field column, right margin —
  computed once and shared. Rows that each compute their own geometry drift.
- **Clip children.** `WS_CLIPCHILDREN`, or the parent paints over its children.
- **Repaint deliberately.** `RedrawWindow` with
  `RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW` after a rebuild.

---

## 5. Verification — the part that decides whether it looks right

Everything above is a rule. This is the process, and it is what actually found the
defects.

### 5.1 Look at the pixels

Take a capture of the window and **look at it**. Not a summary, not a metric: the
image. Several of the defects in this list were obvious instantly in a picture and
invisible in every measurement that was supposed to catch them.

### 5.2 Ask the program, do not infer

- To know where a control **really** is: log every child's rectangle from
  `GetWindowRect`, and print the layout's own numbers beside it, behind an
  environment variable. Where Windows put a control and where the layout thinks it
  put it are different questions and the difference is the bug.
- To know whether controls are themed: `GetWindowTheme` on a live child. It is the
  only direct answer, and it is a different question from "does it have the right
  colours".
- To know the size: `GetClientRect` on the live window.

### 5.3 Use an impossible value to find out what is reachable

A right colour, a plausible size, a sensible default — none of these tell you
whether your code ran. Set the brush to magenta, the size to something absurd, the
text to garbage. Then you know which pixels, which control, which message. This one
technique settled three separate questions in one session.

### 5.4 Distinguish "stale binary" from "wrong code"

An executable that was still running when the build ran **cannot be replaced**. The
build reports success and what opens afterwards is the previous one. Print the
executable's path and its build time next to any observation, or you will compare a
capture of the old build against a measurement of the new one. Doing exactly that,
twice, sent a whole investigation the wrong way.

**And compare like with like.** A window at 100% and the same window at 125% have
different pixel sizes. Any arithmetic that mixes two scales is worthless, and it
looks exactly like a real finding.

### 5.5 Drive the window and check what it saved

A dialog is not verified by looking at it. Send the input, close it, and assert on
what it wrote: type into a field with `WM_CHAR`, press the button, then read the
settings file back and compare. That is what catches a field that renders but does
not store, and a default that does not survive a round trip.

Also drive it at a scale other than 100%, including through a synthetic
`WM_DPICHANGED`, because that path destroys and rebuilds every control and is the
easiest one to break.

### 5.6 Fix the invariant, not the instance

Every defect in this list got a test that would have caught it beside the code
change that fixed it. The test that asserts a container clears its own contents
found a **second** instance of the same mistake that had not been noticed.

---

## 6. What the system will not let you change

Worth knowing, because these look like bugs and are not. Each one cost time:

- **A Win32 check box is a square outline.** Not because the control is old, but
  because the system's button theme contains exactly that: asking the theme API
  directly for `BP_CHECKBOX` returns a 13×13 square. Windows 11's rounded, filled
  check boxes come from a different toolkit and **cannot be borrowed by a Win32
  control**. To have them, the window has to be built with that toolkit.
- **A themed group box paints its own caption background** and ignores the parent's
  answer. The only fix is not to use a group box.
- **`CreateDIBSection` cannot be pointed at memory you own**; it returns
  `E_INVALIDARG`. There is no device context whose pixels are a buffer you already
  have.
- **`PrintWindow` with `PW_RENDERFULLCONTENT` is unreliable** for a window with
  layered or composited children; a screen copy of a moved-to-(0,0) window is
  cleaner.
- **A process cannot replace its own manifest.** It is read before `main`.

---

## 7. The shortest version

1. Manifest first: common controls v6, `supportedOS` list, DPI awareness.
2. Ask for DPI awareness before creating anything, and check the answer.
3. Layout is pure arithmetic in one module, scaled once, tested at five scales.
4. Measure the client area and correct it; do not compute the frame.
5. System colours, owned brushes, released on every path.
6. Labels get no background; group boxes get one; fields get white.
7. Capture, look at the picture, and ask the program where things really are.
8. Use an impossible value to find out whether your code is even reached.
9. Print the build time beside every observation.
10. Give every fixed defect a test.
