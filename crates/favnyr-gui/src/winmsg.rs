//! Windows tab transport using system window messages.
//!
//! Each instance marks and subclasses its main window. WM_COPYDATA copies a
//! bounded payload into owned memory before deferring UI work to Slint.
//! V1 retains the legacy single-tab wire format. V2 separates native delivery
//! from acceptance: the source waits for a reply before removing its tabs.
//! Synchronous sends have a timeout; hover previews use asynchronous messages.

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use tracing::info;

    type Hwnd = isize;

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    struct CopyDataStruct {
        dw_data: usize,
        cb_data: u32,
        lp_data: *const core::ffi::c_void,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct GuiThreadInfo {
        size: u32,
        flags: u32,
        active: Hwnd,
        focus: Hwnd,
        capture: Hwnd,
        menu_owner: Hwnd,
        move_size: Hwnd,
        caret: Hwnd,
        caret_rect: Rect,
    }

    /// Largest visible ownerless window found on the UI thread so far.
    struct MainWindowPick {
        hwnd: Hwnd,
        area: i64,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn WindowFromPoint(pt: Point) -> Hwnd;
        fn GetAncestor(hwnd: Hwnd, flags: u32) -> Hwnd;
        fn SetPropW(hwnd: Hwnd, s: *const u16, data: isize) -> i32;
        fn GetPropW(hwnd: Hwnd, s: *const u16) -> isize;
        // Bound native delivery time. A positive return only acknowledges
        // receipt by the window procedure, not insertion into the target view.
        fn SendMessageTimeoutW(
            hwnd: Hwnd,
            msg: u32,
            wparam: usize,
            lparam: isize,
            flags: u32,
            timeout_ms: u32,
            result: *mut usize,
        ) -> isize;
        fn SetForegroundWindow(hwnd: Hwnd) -> i32;
        fn GetForegroundWindow() -> Hwnd;
        fn SetFocus(hwnd: Hwnd) -> Hwnd;
        fn GetGUIThreadInfo(thread: u32, info: *mut GuiThreadInfo) -> i32;
        fn GetWindowThreadProcessId(hwnd: Hwnd, process: *mut u32) -> u32;
        fn GetAsyncKeyState(key: i32) -> i16;
        fn GetCapture() -> Hwnd;
        fn ReleaseCapture() -> i32;
        fn ClientToScreen(hwnd: Hwnd, point: *mut Point) -> i32;
        fn ScreenToClient(hwnd: Hwnd, point: *mut Point) -> i32;
        fn EnumThreadWindows(
            thread_id: u32,
            cb: unsafe extern "system" fn(Hwnd, isize) -> i32,
            lparam: isize,
        ) -> i32;
        fn IsWindowVisible(hwnd: Hwnd) -> i32;
        fn GetWindow(hwnd: Hwnd, cmd: u32) -> Hwnd;
        fn GetWindowRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
        fn GetWindowTextW(hwnd: Hwnd, buf: *mut u16, count: i32) -> i32;
        fn GetWindowTextLengthW(hwnd: Hwnd) -> i32;
        fn GetClassNameW(hwnd: Hwnd, buf: *mut u16, count: i32) -> i32;
        // Cross-instance hover: ASYNCHRONOUS (fire-and-forget) → adds no
        // latency to the source's drag (unlike SendMessage).
        fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
        fn RegisterWindowMessageW(s: *const u16) -> u32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadId() -> u32;
    }
    // Window subclassing (Common Controls v6, system DLL).
    #[link(name = "comctl32")]
    unsafe extern "system" {
        fn SetWindowSubclass(
            hwnd: Hwnd,
            proc: unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize,
            id: usize,
            ref_data: usize,
        ) -> i32;
        fn DefSubclassProc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    }

    const WM_COPYDATA: u32 = 0x004A;
    const WM_DEVICECHANGE: u32 = 0x0219;
    const WM_POWERBROADCAST: u32 = 0x0218;
    /// Resume subtypes of `WM_POWERBROADCAST`. Both are broadcast to every
    /// top-level window with no prior registration. Two of them because they
    /// describe different ways back: `RESUMESUSPEND` follows a resume the user
    /// asked for, `RESUMEAUTOMATIC` one the machine decided on — and a laptop
    /// waking to its lock screen reports the second.
    const PBT_APMRESUMESUSPEND: usize = 0x0007;
    const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
    /// `DBT_DEVNODES_CHANGED`: a device was added to or removed from the
    /// machine. Windows broadcasts it to every top-level window WITHOUT any
    /// prior `RegisterDeviceNotification`, and it carries exactly the
    /// granularity needed here — "something was plugged or unplugged", not
    /// which device — which is the signal to re-scan rather than to poll.
    const DBT_DEVNODES_CHANGED: usize = 0x0007;
    const GA_ROOT: u32 = 2;
    const GW_OWNER: u32 = 4;
    /// Subclass identity, unique to this application among the subclasses set
    /// on the same window. The value is the ASCII of the application name,
    /// spelled out in the comment so that renaming the application finds it by
    /// searching for the old name: a value written only in hexadecimal is
    /// invisible to that search, and outlives the rename unnoticed.
    ///
    /// Four bytes, so the value also fits a 32-bit `usize`: the identity does
    /// not depend on the pointer width the application is built for.
    const SUBCLASS_ID: usize = 0x46_56_4E_52; // "FVNR", for FAVNYR
    /// Wire tags remain distinct so older windows can still receive compatible
    /// single tabs. The window property advertises batch support separately.
    const PROTO_TAB_V1: usize = 0x46_56_4E_01; // "FVN", for FAVNYR, + version 1
    const PROTO_BATCH_V2: usize = 0x46_56_4E_02;
    const SMTO_BLOCK: u32 = 0x0001;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    const SEND_TIMEOUT_MS: u32 = 2000;

    /// Property name marking a Favnyr window (detectable by other instances).
    /// NUL-terminated (UTF-16).
    fn prop_name() -> Vec<u16> {
        "FavnyrTabTargetWindow\0".encode_utf16().collect()
    }

    // HWND of THIS instance's main window (0 = not initialized). Used to
    // exclude itself from detection and to identify itself as the sender.
    static SELF_HWND: AtomicIsize = AtomicIsize::new(0);
    // REGISTERED window message IDs (RegisterWindowMessageW → same values
    // across all Favnyr instances for a given string). Resolved at init.
    static WM_HOVER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    static WM_HOVER_END: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    static WM_BATCH_BEGIN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    static WM_BATCH_HOVER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    static HOVER_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

    /// Incoming message: from another Favnyr instance, or from the system.
    pub enum Incoming {
        /// Tab DROPPED (serialized payload) → insert it.
        Transfer(String),
        Batch(isize, String),
        BatchHover(i32, i32, usize),
        /// HOVER in progress at SCREEN point `(x, y)` (physical) → insertion preview.
        Hover(i32, i32),
        /// End of hover (cursor left / drag ended elsewhere) → clear the preview.
        HoverEnd,
        /// The set of devices attached to the machine changed → re-scan the
        /// portable devices. Sent by Windows, not by another instance.
        DevicesChanged,
        /// The machine came back from sleep or hibernation. Sent by Windows.
        /// Long-lived per-window registrations are re-asserted at this point:
        /// the drop target survives an ordinary session, but a resume is where
        /// it has been observed to stop being consulted.
        PowerResumed,
    }

    /// Handler for incoming messages (on the UI thread).
    type Handler = Box<dyn Fn(Incoming)>;

    thread_local! {
        /// Handler being called (UI thread). Captured once at init; never
        /// crosses a thread (thread_local of the UI thread).
        static HANDLER: RefCell<Option<Handler>> = const { RefCell::new(None) };
    }

    fn hover_msg_name() -> Vec<u16> {
        "FavnyrTabHover\0".encode_utf16().collect()
    }
    fn hover_end_msg_name() -> Vec<u16> {
        "FavnyrTabHoverEnd\0".encode_utf16().collect()
    }

    /// Posts (UI thread) an `Incoming` variant to the handler, deferred.
    fn dispatch(incoming: Incoming) -> bool {
        // We're already on the UI thread (WndProc), but we DEFER to avoid
        // re-entering Slint from the message loop (consistent with the
        // transfer, and safe for a synchronous WM_COPYDATA).
        // `invoke_from_event_loop` requires `Send` → `Incoming` only captures
        // `Send` types.
        slint::invoke_from_event_loop(move || {
            HANDLER.with(|h| {
                if let Some(f) = h.borrow().as_ref() {
                    f(incoming);
                }
            });
        })
        .is_ok()
    }

    /// EnumThreadWindows already excludes child windows. Restrict its results
    /// to visible windows without an owner before comparing their surfaces.
    unsafe fn is_candidate_window(hwnd: Hwnd) -> bool {
        unsafe { IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER) == 0 }
    }

    /// A UI thread can also own small visible service windows. Examine every
    /// candidate so their enumeration order cannot decide where IPC and OLE
    /// are registered. Class names and implementation-specific styles are
    /// deliberately not selection criteria.
    ///
    /// Two traits are required rather than one, because the scan runs on a
    /// retrying timer whose first attempt can land before the application
    /// window is on screen. A positive area does not rule a service window
    /// out, as it has one too, so the area alone would answer the service
    /// window whenever it is the only candidate, and that answer is final: the
    /// caller stops retrying and the subclass, the inter-instance marking and
    /// the drop target stay on it for the whole session. Finding nothing is
    /// harmless by comparison, since it only costs one more attempt.
    unsafe extern "system" fn pick_main_cb(hwnd: Hwnd, lparam: isize) -> i32 {
        unsafe {
            if !is_candidate_window(hwnd) {
                return 1;
            }
            // The window the application named is the one it put on screen. A
            // service window carries no title, so an untitled candidate is
            // passed over even when nothing else is there.
            if GetWindowTextLengthW(hwnd) == 0 {
                return 1;
            }
            let mut rect = Rect::default();
            if GetWindowRect(hwnd, &mut rect) == 0 {
                return 1;
            }
            let width = (i64::from(rect.right) - i64::from(rect.left)).max(0);
            let height = (i64::from(rect.bottom) - i64::from(rect.top)).max(0);
            let area = width.saturating_mul(height);
            let pick = &mut *(lparam as *mut MainWindowPick);
            if area > pick.area {
                pick.hwnd = hwnd;
                pick.area = area;
            }
            1 // Every candidate must be examined.
        }
    }

    /// This thread's main window, or `None` while no titled window of its own
    /// has a surface yet, which leaves initialization retryable.
    fn pick_main_window() -> Option<Hwnd> {
        let mut pick = MainWindowPick { hwnd: 0, area: 0 };
        unsafe {
            EnumThreadWindows(
                GetCurrentThreadId(),
                pick_main_cb,
                &mut pick as *mut MainWindowPick as isize,
            );
        }
        (pick.area > 0).then_some(pick.hwnd)
    }

    /// Records the candidates once after successful initialization. Their
    /// dimensions and selection make a registration on a service window
    /// diagnosable even when no drag callback ever reaches the application.
    fn describe_thread_windows(picked: Hwnd) {
        unsafe {
            EnumThreadWindows(GetCurrentThreadId(), describe_window_cb, picked);
        }
    }

    unsafe extern "system" fn describe_window_cb(hwnd: Hwnd, lparam: isize) -> i32 {
        unsafe {
            if !is_candidate_window(hwnd) {
                return 1;
            }
            let mut rect = Rect::default();
            let rect_available = GetWindowRect(hwnd, &mut rect) != 0;
            let mut buffer = [0u16; 128];
            let read = GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
            let class = String::from_utf16_lossy(&buffer[..read.max(0) as usize]);
            let read = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
            let title = String::from_utf16_lossy(&buffer[..read.max(0) as usize]);
            info!(
                hwnd,
                class = %class,
                title = %title,
                rect_available,
                left = rect.left,
                top = rect.top,
                width = i64::from(rect.right) - i64::from(rect.left),
                height = i64::from(rect.bottom) - i64::from(rect.top),
                picked = hwnd == lparam,
                "candidate window of the Favnyr UI thread"
            );
            1
        }
    }

    unsafe extern "system" fn subclass_proc(
        hwnd: Hwnd,
        msg: u32,
        wparam: usize,
        lparam: isize,
        _id: usize,
        _data: usize,
    ) -> isize {
        unsafe {
            // Hover (async PostMessage): SCREEN coords packed into wparam/lparam.
            if msg != 0 && msg == WM_BATCH_BEGIN.load(Ordering::Relaxed) {
                HOVER_COUNT.store(
                    wparam.clamp(1, crate::tab_transfer::MAX_TABS),
                    Ordering::Relaxed,
                );
                return 0;
            }
            if msg != 0 && msg == WM_BATCH_HOVER.load(Ordering::Relaxed) {
                dispatch(Incoming::BatchHover(
                    wparam as i32,
                    lparam as i32,
                    HOVER_COUNT.load(Ordering::Relaxed),
                ));
                return 0;
            }
            let hover = WM_HOVER.load(Ordering::Relaxed);
            let hover_end = WM_HOVER_END.load(Ordering::Relaxed);
            if hover != 0 && msg == hover {
                dispatch(Incoming::Hover(wparam as i32, lparam as i32));
                return 0;
            }
            if hover_end != 0 && msg == hover_end {
                dispatch(Incoming::HoverEnd);
                return 0;
            }
            // Device arrival/removal. Observed, never consumed: this is a
            // broadcast, so the default procedure and any other subclass on
            // this window must still receive it.
            if msg == WM_DEVICECHANGE && wparam == DBT_DEVNODES_CHANGED {
                dispatch(Incoming::DevicesChanged);
                return DefSubclassProc(hwnd, msg, wparam, lparam);
            }
            // Resume from sleep. Observed, never consumed, for the same reason
            // as the device broadcast above.
            if msg == WM_POWERBROADCAST
                && (wparam == PBT_APMRESUMESUSPEND || wparam == PBT_APMRESUMEAUTOMATIC)
            {
                dispatch(Incoming::PowerResumed);
                return DefSubclassProc(hwnd, msg, wparam, lparam);
            }
            if msg == WM_COPYDATA {
                if lparam == 0 {
                    return 0;
                }
                let cds = &*(lparam as *const CopyDataStruct);
                if cds.dw_data == PROTO_BATCH_V2 {
                    if !supports_batch(wparam as isize)
                        || cds.lp_data.is_null()
                        || cds.cb_data == 0
                        || cds.cb_data as usize > crate::tab_transfer::MAX_BYTES
                    {
                        return 0;
                    }
                    let bytes =
                        std::slice::from_raw_parts(cds.lp_data as *const u8, cds.cb_data as usize);
                    let Ok(payload) = std::str::from_utf8(bytes) else {
                        return 0;
                    };
                    return isize::from(dispatch(Incoming::Batch(
                        wparam as isize,
                        payload.to_owned(),
                    )));
                }
                // Only payloads marked with OUR protocol are processed: a
                // WM_COPYDATA from a third-party application passes through to
                // DefSubclassProc.
                if cds.dw_data != PROTO_TAB_V1 {
                    return DefSubclassProc(hwnd, msg, wparam, lparam);
                }
                if !cds.lp_data.is_null()
                    && cds.cb_data > 0
                    && cds.cb_data as usize <= crate::tab_transfer::MAX_BYTES
                {
                    let bytes =
                        std::slice::from_raw_parts(cds.lp_data as *const u8, cds.cb_data as usize);
                    let s = String::from_utf8_lossy(bytes).into_owned();
                    // DEFERRED processing on the UI thread (immediate return → the
                    // source isn't left blocked in its synchronous SendMessage).
                    dispatch(Incoming::Transfer(s));
                }
                return 1; // TRUE = message handled
            }
            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
    }

    /// Marks + subclasses this instance's window and registers the handler
    /// for incoming messages. Must be called on the UI thread, AFTER the
    /// window exists (HWND created). Idempotent; `false` = window not found
    /// yet → retry.
    pub fn init(handler: impl Fn(Incoming) + 'static) -> bool {
        if SELF_HWND.load(Ordering::SeqCst) != 0 {
            return true; // already initialized
        }
        let Some(hwnd) = pick_main_window() else {
            // Nothing on screen carries the application's name yet. The timer
            // behind this call keeps trying, so the only cost is one attempt.
            info!("no titled window with a surface yet");
            return false;
        };
        unsafe {
            if SetWindowSubclass(hwnd, subclass_proc, SUBCLASS_ID, 0) == 0 {
                return false; // subclassing refused → IPC inactive, retryable
            }
            SetPropW(hwnd, prop_name().as_ptr(), 2);
            WM_BATCH_BEGIN.store(
                RegisterWindowMessageW(
                    "FavnyrBatchHoverBegin\0"
                        .encode_utf16()
                        .collect::<Vec<_>>()
                        .as_ptr(),
                ),
                Ordering::SeqCst,
            );
            WM_BATCH_HOVER.store(
                RegisterWindowMessageW(
                    "FavnyrBatchHover\0"
                        .encode_utf16()
                        .collect::<Vec<_>>()
                        .as_ptr(),
                ),
                Ordering::SeqCst,
            );
            WM_HOVER.store(
                RegisterWindowMessageW(hover_msg_name().as_ptr()),
                Ordering::SeqCst,
            );
            WM_HOVER_END.store(
                RegisterWindowMessageW(hover_end_msg_name().as_ptr()),
                Ordering::SeqCst,
            );
            SELF_HWND.store(hwnd, Ordering::SeqCst);
        }
        describe_thread_windows(hwnd);
        HANDLER.with(|h| *h.borrow_mut() = Some(Box::new(handler)));
        true
    }

    /// Signals a HOVER in progress at SCREEN point `(x, y)` (physical) to the
    /// target window (async, non-blocking). Coords packed into wparam/lparam.
    pub fn hover(target: isize, x: i32, y: i32) {
        let msg = WM_HOVER.load(Ordering::SeqCst);
        if msg != 0 {
            unsafe { PostMessageW(target, msg, x as isize as usize, y as isize) };
        }
    }

    /// Signals the END of a hover to the target window (clears its preview).
    pub fn hover_end(target: isize) {
        let msg = WM_HOVER_END.load(Ordering::SeqCst);
        if msg != 0 {
            unsafe { PostMessageW(target, msg, 0, 0) };
        }
    }

    pub fn supports_batch(target: isize) -> bool {
        target != 0 && unsafe { GetPropW(target, prop_name().as_ptr()) >= 2 }
    }

    pub fn begin_batch_hover(target: isize, count: usize) {
        let msg = WM_BATCH_BEGIN.load(Ordering::SeqCst);
        if msg != 0 {
            unsafe { PostMessageW(target, msg, count, 0) };
        }
    }

    pub fn batch_hover(target: isize, x: i32, y: i32) {
        let msg = WM_BATCH_HOVER.load(Ordering::SeqCst);
        if msg != 0 {
            unsafe { PostMessageW(target, msg, x as isize as usize, y as isize) };
        }
    }

    /// Favnyr window OTHER than self under the SCREEN point `(x, y)`
    /// (physical). `None` if the point isn't over a third-party Favnyr window.
    pub fn target_at(x: i32, y: i32) -> Option<isize> {
        unsafe {
            let top = GetAncestor(WindowFromPoint(Point { x, y }), GA_ROOT);
            if top == 0 || top == SELF_HWND.load(Ordering::SeqCst) {
                return None;
            }
            if GetPropW(top, prop_name().as_ptr()) != 0 {
                Some(top)
            } else {
                None
            }
        }
    }

    /// Main HWND of this instance, available after [`init`]. Shared with the
    /// OLE file drop target to avoid a second, fragile per-thread window
    /// detection.
    pub fn self_hwnd() -> Option<isize> {
        let hwnd = SELF_HWND.load(Ordering::SeqCst);
        (hwnd != 0).then_some(hwnd)
    }

    /// Converts a point from the Favnyr window's physical CLIENT coordinate
    /// space to screen coordinates. Unlike `Window::position`, this excludes
    /// exactly the Win32 borders and title bar.
    pub fn client_to_screen(x: i32, y: i32) -> Option<(i32, i32)> {
        let hwnd = SELF_HWND.load(Ordering::SeqCst);
        if hwnd == 0 {
            return None;
        }
        let mut point = Point { x, y };
        (unsafe { ClientToScreen(hwnd, &mut point) } != 0).then_some((point.x, point.y))
    }

    /// Inverse conversion, used to transform OLE and IPC screen coordinates
    /// into CLIENT coordinates before the Slint hit-test.
    pub fn screen_to_client(x: i32, y: i32) -> Option<(i32, i32)> {
        let hwnd = SELF_HWND.load(Ordering::SeqCst);
        if hwnd == 0 {
            return None;
        }
        let mut point = Point { x, y };
        (unsafe { ScreenToClient(hwnd, &mut point) } != 0).then_some((point.x, point.y))
    }

    /// Activates the target window after an OLE drop. Without this, the first
    /// click on the drop menu would sometimes only activate the window,
    /// without triggering the chosen entry.
    pub fn focus_self() {
        let hwnd = SELF_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                let activated = SetForegroundWindow(hwnd) != 0;
                SetFocus(hwnd);
                tracing::debug!(
                    activated,
                    foreground = GetForegroundWindow() == hwnd,
                    "OLE drop window activation"
                );
            }
        }
    }

    pub fn left_button_down() -> bool {
        unsafe { GetAsyncKeyState(1) < 0 }
    }

    /// Remember the source before activation changes the foreground thread.
    pub fn drop_source_thread() -> u32 {
        unsafe { GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut()) }
    }

    fn native_drop_released(source: u32) -> bool {
        unsafe {
            if [1, 2, 4].iter().any(|key| GetAsyncKeyState(*key) < 0) {
                return false;
            }
            let own = SELF_HWND.load(Ordering::SeqCst);
            for thread in [source, 0] {
                let mut info = GuiThreadInfo {
                    size: std::mem::size_of::<GuiThreadInfo>() as u32,
                    ..Default::default()
                };
                if GetGUIThreadInfo(thread, &mut info) != 0
                    && info.capture != 0
                    && GetAncestor(info.capture, GA_ROOT) != own
                {
                    return false;
                }
            }
            // Only release capture owned by this window, after all buttons are
            // up. ReleaseCapture cannot release another thread's OLE capture.
            let capture = GetCapture();
            if own != 0 && capture != 0 && GetAncestor(capture, GA_ROOT) == own {
                ReleaseCapture();
            }
            true
        }
    }

    /// OLE can pump timers before its source unwinds. Retry only for this drop;
    /// no native polling remains after delivery or the bounded timeout.
    pub fn after_native_drop(source: u32, delivery: impl FnOnce(bool) + 'static) {
        fn schedule(source: u32, attempts: u8, delivery: Box<dyn FnOnce(bool)>) {
            slint::Timer::single_shot(std::time::Duration::from_millis(16), move || {
                if native_drop_released(source) {
                    focus_self();
                    delivery(true);
                } else if attempts > 1 {
                    schedule(source, attempts - 1, delivery);
                } else {
                    tracing::warn!(source_thread = source, "native drop input did not settle");
                    delivery(false);
                }
            });
        }
        schedule(source, 60, Box::new(delivery));
    }

    /// Delivers owned UTF-8 bytes with a bounded wait for the native handler.
    /// A positive result alone never proves that the target inserted any tabs.
    fn send_message(target: isize, payload: &str, protocol: usize) -> bool {
        if payload.is_empty() || payload.len() > crate::tab_transfer::MAX_BYTES {
            return false;
        }
        let bytes = payload.as_bytes();
        let cds = CopyDataStruct {
            dw_data: protocol,
            cb_data: bytes.len() as u32,
            lp_data: bytes.as_ptr() as *const core::ffi::c_void,
        };
        unsafe {
            let self_hwnd = SELF_HWND.load(Ordering::SeqCst) as usize;
            let mut lresult: usize = 0;
            let rc = SendMessageTimeoutW(
                target,
                WM_COPYDATA,
                self_hwnd,
                &cds as *const CopyDataStruct as isize,
                SMTO_BLOCK | SMTO_ABORTIFHUNG,
                SEND_TIMEOUT_MS,
                &mut lresult,
            );
            if rc == 0 || lresult != 1 {
                return false; // target frozen / gone / message not handled
            }
        }
        true
    }

    pub fn send(target: isize, payload: &str) -> bool {
        let sent = send_message(target, payload, PROTO_TAB_V1);
        if sent {
            unsafe { SetForegroundWindow(target) };
        }
        sent
    }

    /// A positive result means queued, not accepted. Only a V2 reply commits removal.
    pub fn send_batch(target: isize, payload: &str) -> bool {
        supports_batch(target) && send_message(target, payload, PROTO_BATCH_V2)
    }
}

#[cfg(not(windows))]
mod imp {
    /// Incoming message (inert variant outside Windows). The variants are
    /// filtered (`match`) by the handler but never CONSTRUCTED until
    /// cross-instance IPC is implemented on Linux → `allow(dead_code)`.
    #[allow(dead_code)]
    pub enum Incoming {
        Transfer(String),
        Batch(isize, String),
        BatchHover(i32, i32, usize),
        Hover(i32, i32),
        HoverEnd,
        DevicesChanged,
        PowerResumed,
    }
    /// No-op outside Windows (`true` = don't retry). Cross-instance IPC is
    /// Windows-only for now (Linux: window detection + IPC still to be defined).
    pub fn init(_handler: impl Fn(Incoming) + 'static) -> bool {
        true
    }
    pub fn target_at(_x: i32, _y: i32) -> Option<isize> {
        None
    }
    pub fn send(_target: isize, _payload: &str) -> bool {
        false
    }
    pub fn supports_batch(_target: isize) -> bool {
        false
    }
    pub fn send_batch(_target: isize, _payload: &str) -> bool {
        false
    }
    pub fn begin_batch_hover(_target: isize, _count: usize) {}
    pub fn batch_hover(_target: isize, _x: i32, _y: i32) {}
    pub fn hover(_target: isize, _x: i32, _y: i32) {}
    pub fn hover_end(_target: isize) {}
    // `self_hwnd` / `client_to_screen` / `screen_to_client` / `focus_self`: no
    // stub outside Windows — these window helpers (HWND, coordinate
    // conversions, focus) are only called under `#[cfg(windows)]` (OLE file
    // drag, screen↔client conversions). On Linux, `Window::position()` serves
    // as the fallback directly in the bridge (`window_logical_to_screen`, etc.).
}

pub use imp::{
    Incoming, batch_hover, begin_batch_hover, hover, hover_end, init, send, send_batch,
    supports_batch, target_at,
};
#[cfg(windows)]
pub use imp::{
    after_native_drop, client_to_screen, drop_source_thread, focus_self, left_button_down,
    screen_to_client, self_hwnd,
};
