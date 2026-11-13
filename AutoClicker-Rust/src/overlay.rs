//! Countdown-Rahmen über dem ganzen Bildschirm ("NICHT BEWEGEN").
//! Natives, klickdurchlässiges Layered-Window (ersetzt das Tkinter-Fenster):
//! kein zusätzlicher GUI-Toolkit-Start, und im versteckten Zustand 0 % CPU (Timer ist aus).

use crate::win::*;
use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

/// Transparenzfarbe (COLORREF 0x00BBGGRR für #010203)
const KEY_COLOR: u32 = 0x0003_0201;
const COLOR_PURPLE: u32 = 0x00F7_55A8; // #a855f7
const COLOR_ORANGE: u32 = 0x003C_92FB; // #fb923c
const COLOR_WHITE: u32 = 0x00FF_FFFF;

const WM_SHOW_OVERLAY: u32 = WM_APP + 1;
const WM_HIDE_OVERLAY: u32 = WM_APP + 2;
const TIMER_ID: usize = 1;
const TICK_MS: u32 = 40;

static END: Mutex<Option<Instant>> = Mutex::new(None);
static PHASE: AtomicU32 = AtomicU32::new(0);

fn end_time() -> Option<Instant> {
    *END.lock().unwrap_or_else(|e| e.into_inner())
}

fn set_end_time(value: Option<Instant>) {
    *END.lock().unwrap_or_else(|e| e.into_inner()) = value;
}

pub struct Overlay {
    hwnd: HWND,
}

impl Overlay {
    pub fn new() -> Result<Overlay, String> {
        let (tx, rx) = mpsc::channel::<Result<HWND, String>>();
        std::thread::Builder::new()
            .name("overlay".into())
            .spawn(move || unsafe { run(tx) })
            .map_err(|e| format!("Der Countdown-Rahmen konnte nicht gestartet werden: {e}"))?;
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(hwnd)) => Ok(Overlay { hwnd }),
            Ok(Err(e)) => Err(format!("Der Countdown-Rahmen konnte nicht gestartet werden: {e}")),
            Err(_) => Err("Der Countdown-Rahmen konnte nicht gestartet werden.".into()),
        }
    }

    pub fn show(&self, secs: u64) {
        set_end_time(Some(Instant::now() + Duration::from_secs(secs)));
        unsafe {
            PostMessageW(self.hwnd, WM_SHOW_OVERLAY, 0, 0);
        }
    }

    /// Versteckt das Fenster sofort (auch aus einem anderen Thread).
    pub fn hide(&self) {
        set_end_time(None);
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
            PostMessageW(self.hwnd, WM_HIDE_OVERLAY, 0, 0);
        }
    }
}

fn screen_rect() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

unsafe fn run(tx: mpsc::Sender<Result<HWND, String>>) {
    let module = GetModuleHandleW(std::ptr::null());
    let class = wide("WardogsCountdownOverlay");
    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: module,
        hIcon: 0,
        hCursor: 0,
        hbrBackground: 0,
        lpszMenuName: std::ptr::null(),
        lpszClassName: class.as_ptr(),
    };
    if RegisterClassW(&wc) == 0 {
        let _ = tx.send(Err("Fensterklasse konnte nicht registriert werden".into()));
        return;
    }
    let (left, top, width, height) = screen_rect();
    let hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class.as_ptr(),
        class.as_ptr(),
        WS_POPUP,
        left,
        top,
        width,
        height,
        0,
        0,
        module,
        std::ptr::null(),
    );
    if hwnd == 0 {
        let _ = tx.send(Err("Fenster konnte nicht erstellt werden".into()));
        return;
    }
    SetLayeredWindowAttributes(hwnd, KEY_COLOR, 0, LWA_COLORKEY);
    let _ = tx.send(Ok(hwnd));

    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, 0, 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_SHOW_OVERLAY => {
            let (left, top, width, height) = screen_rect();
            PHASE.store(0, SeqCst);
            SetWindowPos(hwnd, HWND_TOPMOST, left, top, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            SetTimer(hwnd, TIMER_ID, TICK_MS, std::ptr::null());
            paint(hwnd);
            0
        }
        WM_HIDE_OVERLAY => {
            KillTimer(hwnd, TIMER_ID);
            ShowWindow(hwnd, SW_HIDE);
            0
        }
        WM_TIMER => {
            match end_time() {
                Some(end) if Instant::now() < end => paint(hwnd),
                _ => {
                    KillTimer(hwnd, TIMER_ID);
                    ShowWindow(hwnd, SW_HIDE);
                }
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            BeginPaint(hwnd, &mut ps);
            EndPaint(hwnd, &ps);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

// ---------------------------------------------------------------
//  Zeichnen (Double-Buffer, einmal angelegt)
// ---------------------------------------------------------------
struct Gfx {
    dc: HDC,
    w: i32,
    h: i32,
    key: HGDI,
    purple: HGDI,
    orange: HGDI,
    big: HGDI,
    small: HGDI,
}

thread_local! {
    static GFX: RefCell<Option<Gfx>> = const { RefCell::new(None) };
}

unsafe fn create_gfx(screen: HDC, w: i32, h: i32) -> Gfx {
    let dc = CreateCompatibleDC(screen);
    let bmp = CreateCompatibleBitmap(screen, w, h);
    SelectObject(dc, bmp);
    SetBkMode(dc, BKMODE_TRANSPARENT);
    SetTextAlign(dc, TA_CENTER);
    SelectObject(dc, GetStockObject(NULL_PEN));
    let face = wide("Segoe UI");
    let font = |px: i32| {
        CreateFontW(
            -px,
            0,
            0,
            0,
            FW_BOLD,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            0,
            0,
            ANTIALIASED_QUALITY,
            0,
            face.as_ptr(),
        )
    };
    Gfx {
        dc,
        w,
        h,
        key: CreateSolidBrush(KEY_COLOR),
        purple: CreateSolidBrush(COLOR_PURPLE),
        orange: CreateSolidBrush(COLOR_ORANGE),
        big: font((h / 11).max(40)),
        small: font((h / 40).max(14)),
    }
}

unsafe fn paint(hwnd: HWND) {
    let (_, _, width, height) = screen_rect();
    if width <= 0 || height <= 0 {
        return;
    }
    GFX.with(|cell| {
        let screen = GetDC(hwnd);
        if screen == 0 {
            return;
        }
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(create_gfx(screen, width, height));
        }
        if let Some(gfx) = slot.as_ref() {
            draw(gfx);
            BitBlt(screen, 0, 0, gfx.w, gfx.h, gfx.dc, 0, 0, SRCCOPY);
        }
        ReleaseDC(hwnd, screen);
    });
}

unsafe fn draw(gfx: &Gfx) {
    let rect = RECT { left: 0, top: 0, right: gfx.w, bottom: gfx.h };
    FillRect(gfx.dc, &rect, gfx.key);

    // Laufende Perlenkette am Bildschirmrand
    let (width, height) = (gfx.w as f64, gfx.h as f64);
    let inset = 12.0;
    let radius = 6.0;
    let side_w = width - 2.0 * inset;
    let side_h = height - 2.0 * inset;
    let perimeter = 2.0 * (width + height - 4.0 * inset);
    let phase = PHASE.fetch_add(1, SeqCst) % 40;
    for i in 0..64 {
        let pos = (i as f64 * perimeter / 64.0 + phase as f64 * perimeter / 40.0) % perimeter;
        let (x, y) = if pos < side_w {
            (inset + pos, inset)
        } else if pos < side_w + side_h {
            (width - inset, inset + pos - side_w)
        } else if pos < 2.0 * side_w + side_h {
            (width - inset - (pos - (side_w + side_h)), height - inset)
        } else {
            (inset, height - inset - (pos - (2.0 * side_w + side_h)))
        };
        SelectObject(gfx.dc, if i % 2 == 0 { gfx.purple } else { gfx.orange });
        Ellipse(
            gfx.dc,
            (x - radius) as i32,
            (y - radius) as i32,
            (x + radius) as i32,
            (y + radius) as i32,
        );
    }

    // Zahl + Hinweis in der Mitte
    let remaining = match end_time() {
        Some(end) => {
            let left = end.saturating_duration_since(Instant::now()).as_secs_f64();
            (left.ceil() as i64).max(1)
        }
        None => 1,
    };
    let (cx, cy) = (gfx.w / 2, gfx.h / 2);
    let number = wide(&remaining.to_string());
    let label = wide("NICHT BEWEGEN");
    let big_px = (gfx.h / 11).max(40);
    let small_px = (gfx.h / 40).max(14);

    SelectObject(gfx.dc, gfx.big);
    SetTextColor(gfx.dc, COLOR_WHITE);
    TextOutW(gfx.dc, cx, cy - 20 - big_px * 2 / 3, number.as_ptr(), (number.len() - 1) as i32);

    SelectObject(gfx.dc, gfx.small);
    SetTextColor(gfx.dc, COLOR_ORANGE);
    TextOutW(gfx.dc, cx, cy + 54 - small_px * 2 / 3, label.as_ptr(), (label.len() - 1) as i32);
}
