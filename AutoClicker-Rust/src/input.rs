//! Maus/Tastatur senden (SendInput) und globale Hotkeys (Low-Level-Hooks).

use crate::win::*;
use std::mem::size_of;
use std::sync::mpsc::Sender;
use std::sync::OnceLock;

// Scancodes (funktionieren zuverlässig in Minecraft)
pub const SC_S: u16 = 0x1F;
pub const SC_A: u16 = 0x1E;
pub const SC_D: u16 = 0x20;
pub const SC_LSHIFT: u16 = 0x2A;
pub const SC_LCTRL: u16 = 0x1D;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    Left,
    Middle,
    Right,
}

fn mouse_input(flags: u32) -> INPUT {
    INPUT {
        kind: INPUT_MOUSE,
        u: INPUT_UNION {
            mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

/// Ein kompletter Klick (Down + Up) in einem einzigen Systemaufruf.
pub fn click(button: Button) {
    let (down, up) = match button {
        Button::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        Button::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
    };
    let inputs = [mouse_input(down), mouse_input(up)];
    unsafe {
        SendInput(2, inputs.as_ptr(), size_of::<INPUT>() as i32);
    }
}

pub fn send_scancode(scan: u16, down: bool) {
    let flags = KEYEVENTF_SCANCODE | if down { 0 } else { KEYEVENTF_KEYUP };
    let input = INPUT {
        kind: INPUT_KEYBOARD,
        u: INPUT_UNION { ki: KEYBDINPUT { wVk: 0, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    unsafe {
        SendInput(1, &input, size_of::<INPUT>() as i32);
    }
}

// ---------------------------------------------------------------
//  Globale Hotkeys
// ---------------------------------------------------------------
pub struct RawEvent {
    pub kid: String,
    pub pressed: bool,
}

static SENDER: OnceLock<Sender<RawEvent>> = OnceLock::new();

fn emit(kid: String, pressed: bool) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.send(RawEvent { kid, pressed });
    }
}

unsafe extern "system" fn kbd_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 {
        let k = &*(l as *const KBDLLHOOKSTRUCT);
        // 0x10 = LLKHF_INJECTED: eigene SendInput-Events ignorieren
        if k.flags & 0x10 == 0 {
            let msg = w as u32;
            match msg {
                0x0100 | 0x0104 => emit(key_id(k.vkCode), true),
                0x0101 | 0x0105 => emit(key_id(k.vkCode), false),
                _ => {}
            }
        }
    }
    CallNextHookEx(0, code, w, l)
}

unsafe extern "system" fn mouse_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 {
        let m = &*(l as *const MSLLHOOKSTRUCT);
        // 1 = LLMHF_INJECTED
        if m.flags & 1 == 0 {
            let msg = w as u32;
            let hit: Option<(&str, bool)> = match msg {
                0x0201 => Some(("left", true)),
                0x0202 => Some(("left", false)),
                0x0204 => Some(("right", true)),
                0x0205 => Some(("right", false)),
                0x0207 => Some(("middle", true)),
                0x0208 => Some(("middle", false)),
                0x020B | 0x020C => {
                    let which = (m.mouseData >> 16) & 0xFFFF;
                    Some((if which == 1 { "x1" } else { "x2" }, msg == 0x020B))
                }
                _ => None, // WM_MOUSEMOVE & Co. sofort verwerfen
            };
            if let Some((name, pressed)) = hit {
                emit(format!("Button.{name}"), pressed);
            }
        }
    }
    CallNextHookEx(0, code, w, l)
}

/// Startet einen Thread mit den beiden Hooks und eigener Nachrichtenschleife.
pub fn start_hooks(tx: Sender<RawEvent>) {
    let _ = SENDER.set(tx);
    std::thread::Builder::new()
        .name("hooks".into())
        .spawn(|| unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(kbd_proc), module, 0);
            SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        })
        .expect("Hook-Thread konnte nicht gestartet werden");
}

// ---------------------------------------------------------------
//  Tasten-IDs (kompatibel zur alten Python-Version: "Key.f8", "char:a", "Button.x1")
// ---------------------------------------------------------------
pub fn key_id(vk: u32) -> String {
    match vk {
        0x41..=0x5A => format!("char:{}", (vk as u8 + 32) as char), // a-z
        0x30..=0x39 => format!("char:{}", vk as u8 as char),        // 0-9
        0x70..=0x87 => format!("Key.f{}", vk - 0x6F),               // F1-F24
        0x60..=0x69 => format!("Key.num{}", vk - 0x60),
        _ => match named_key(vk) {
            Some(name) => format!("Key.{name}"),
            None => format!("vk:{vk}"),
        },
    }
}

fn named_key(vk: u32) -> Option<&'static str> {
    Some(match vk {
        0x08 => "backspace",
        0x09 => "tab",
        0x0D => "enter",
        0x13 => "pause",
        0x14 => "caps_lock",
        0x1B => "esc",
        0x20 => "space",
        0x21 => "page_up",
        0x22 => "page_down",
        0x23 => "end",
        0x24 => "home",
        0x25 => "left",
        0x26 => "up",
        0x27 => "right",
        0x28 => "down",
        0x2C => "print_screen",
        0x2D => "insert",
        0x2E => "delete",
        0x5B => "cmd_l",
        0x5C => "cmd_r",
        0x6A => "num_multiply",
        0x6B => "num_add",
        0x6D => "num_subtract",
        0x6E => "num_decimal",
        0x6F => "num_divide",
        0x90 => "num_lock",
        0x91 => "scroll_lock",
        0xA0 => "shift_l",
        0xA1 => "shift_r",
        0xA2 => "ctrl_l",
        0xA3 => "ctrl_r",
        0xA4 => "alt_l",
        0xA5 => "alt_r",
        _ => return None,
    })
}

pub fn key_display(kid: &str) -> String {
    if let Some(name) = kid.strip_prefix("Button.") {
        return match name {
            "x1" => "Maustaste 4".to_string(),
            "x2" => "Maustaste 5".to_string(),
            other => format!("Maus {}", other.to_uppercase()),
        };
    }
    if let Some(name) = kid.strip_prefix("Key.") {
        return name.to_uppercase().replace('_', " ");
    }
    if let Some(ch) = kid.strip_prefix("char:") {
        return ch.to_uppercase();
    }
    if let Some(vk) = kid.strip_prefix("vk:") {
        return format!("Taste {vk}");
    }
    kid.to_uppercase()
}
