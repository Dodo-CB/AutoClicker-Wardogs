//! Logik des Autoklickers (1:1 Port von `Api` aus der Python-Version).

use crate::input::{self, Button, SC_A, SC_D, SC_LCTRL, SC_LSHIFT, SC_S};
use crate::matcher::{self, Template};
use crate::overlay::Overlay;
use crate::{capture, win};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicU8};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

const MATCH_SCALES: [f32; 8] = [0.65, 0.8, 0.9, 1.0, 1.1, 1.25, 1.45, 1.7];
const MATCH_THRESHOLD: f64 = 0.92;
const COUNTDOWN_SECONDS: u64 = 3;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

// ---------------------------------------------------------------
//  Typen
// ---------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Feature {
    Wardogs = 0,
    Clicker = 1,
    Bridge = 2,
}

impl Feature {
    pub fn parse(s: &str) -> Option<Feature> {
        match s {
            "wardogs" => Some(Feature::Wardogs),
            "clicker" => Some(Feature::Clicker),
            "bridge" => Some(Feature::Bridge),
            _ => None,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Feature::Wardogs => "wardogs",
            Feature::Clicker => "clicker",
            Feature::Bridge => "bridge",
        }
    }
    fn from_code(code: u8) -> Option<Feature> {
        match code {
            1 => Some(Feature::Wardogs),
            2 => Some(Feature::Clicker),
            3 => Some(Feature::Bridge),
            _ => None,
        }
    }
    fn code(self) -> u8 {
        self as u8 + 1
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Team {
    Manticore,
    Valkyr,
    Lonesta,
}

impl Team {
    fn parse(s: &str) -> Option<Team> {
        match s {
            "manticore" => Some(Team::Manticore),
            "valkyr" => Some(Team::Valkyr),
            "lonesta" => Some(Team::Lonesta),
            _ => None,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Team::Manticore => "manticore",
            Team::Valkyr => "valkyr",
            Team::Lonesta => "lonesta",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Team::Manticore => "Manticore",
            Team::Valkyr => "Valkyra",
            Team::Lonesta => "Lonestar",
        }
    }
    fn file(self) -> String {
        format!("{}.png", self.name())
    }
    pub fn png(self) -> &'static [u8] {
        match self {
            Team::Manticore => include_bytes!("../assets/teams/Manticore.png"),
            Team::Valkyr => include_bytes!("../assets/teams/Valkyra.png"),
            Team::Lonesta => include_bytes!("../assets/teams/Lonestar.png"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Toggle,
    Hold,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SneakKey {
    Ctrl,
    Shift,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Off,
    A,
    D,
}

#[derive(Clone)]
struct Config {
    interval_ms: u64,
    sneak_ms: u64,
    walk_ms: u64,
    button: Button,
    sneak_key: SneakKey,
    side: Side,
    team: Team,
    mode: Mode,
}

// ---------------------------------------------------------------
//  Engine
// ---------------------------------------------------------------
pub struct Engine {
    cfg: Mutex<Config>,
    hotkeys: Mutex<[String; 3]>,
    active: AtomicU8,
    capturing: AtomicU8,
    clicks: AtomicU64,
    blocks: AtomicU64,
    wardogs_clicks: AtomicU64,
    status: Mutex<String>,
    deadline: Mutex<Option<Instant>>,
    overlay: Mutex<Option<Overlay>>,
    templates: Mutex<HashMap<Team, Arc<Template>>>,
    pressed: Mutex<HashSet<String>>,
    maximized: AtomicBool,
    wake: (Mutex<bool>, Condvar),
    // Fast-Bridge: welche Tasten sind gerade von uns gedrückt?
    s_down: AtomicBool,
    sneak_down: AtomicBool,
    side_down: AtomicU16, // 0 = keine
    sneak_scan: AtomicU16,
}

impl Engine {
    pub fn new() -> Arc<Engine> {
        Arc::new(Engine {
            cfg: Mutex::new(Config {
                interval_ms: 100,
                sneak_ms: 150,
                walk_ms: 150,
                button: Button::Left,
                sneak_key: SneakKey::Ctrl,
                side: Side::Off,
                team: Team::Manticore,
                mode: Mode::Toggle,
            }),
            hotkeys: Mutex::new(["Key.f8".to_string(), "Key.f6".to_string(), "Key.f7".to_string()]),
            active: AtomicU8::new(0),
            capturing: AtomicU8::new(0),
            clicks: AtomicU64::new(0),
            blocks: AtomicU64::new(0),
            wardogs_clicks: AtomicU64::new(0),
            status: Mutex::new("Bereit".to_string()),
            deadline: Mutex::new(None),
            overlay: Mutex::new(None),
            templates: Mutex::new(HashMap::new()),
            pressed: Mutex::new(HashSet::new()),
            maximized: AtomicBool::new(false),
            wake: (Mutex::new(false), Condvar::new()),
            s_down: AtomicBool::new(false),
            sneak_down: AtomicBool::new(false),
            side_down: AtomicU16::new(0),
            sneak_scan: AtomicU16::new(SC_LSHIFT),
        })
    }

    /// Startet Hotkey-Hooks, Eingabe-Verteiler und Arbeits-Thread.
    pub fn start(self: &Arc<Self>) {
        let (tx, rx) = std::sync::mpsc::channel();
        input::start_hooks(tx);

        let me = self.clone();
        thread::Builder::new()
            .name("input-dispatch".into())
            .spawn(move || {
                for ev in rx {
                    me.on_input(ev.kid, ev.pressed);
                }
            })
            .expect("Eingabe-Thread");

        let me = self.clone();
        thread::Builder::new()
            .name("worker".into())
            .spawn(move || me.worker())
            .expect("Arbeits-Thread");
    }

    // ---------- kleine Helfer ----------
    fn cfg(&self) -> Config {
        lock(&self.cfg).clone()
    }
    fn active(&self) -> Option<Feature> {
        Feature::from_code(self.active.load(SeqCst))
    }
    fn set_active(&self, f: Option<Feature>) {
        self.active.store(f.map_or(0, Feature::code), SeqCst);
    }
    fn is_capturing(&self) -> bool {
        self.capturing.load(SeqCst) != 0
    }
    fn set_status(&self, s: impl Into<String>) {
        *lock(&self.status) = s.into();
    }
    fn wake(&self) {
        let (m, cv) = &self.wake;
        *lock(m) = true;
        cv.notify_all();
    }
    pub fn set_maximized(&self, v: bool) {
        self.maximized.store(v, SeqCst);
    }

    /// Blockiert, bis `wake()` gerufen wird.
    fn idle_wait(&self) {
        let (m, cv) = &self.wake;
        let mut flag = lock(m);
        while !*flag {
            flag = cv.wait(flag).unwrap_or_else(|e| e.into_inner());
        }
        *flag = false;
    }

    /// Wartet höchstens `dur` oder bis `wake()` gerufen wird.
    fn wait_wake(&self, dur: Duration) {
        let (m, cv) = &self.wake;
        let flag = lock(m);
        let (mut flag, _) = cv
            .wait_timeout_while(flag, dur, |woken| !*woken)
            .unwrap_or_else(|e| e.into_inner());
        *flag = false;
    }

    /// Wartet `millis`. Gibt false zurück, wenn zwischendurch gestoppt wurde.
    fn wait(&self, millis: u64, feat: Feature) -> bool {
        let end = Instant::now() + ms(millis);
        loop {
            if self.active() != Some(feat) || self.is_capturing() {
                return false;
            }
            let now = Instant::now();
            if now >= end {
                return true;
            }
            self.wait_wake(end - now);
        }
    }

    // ---------- Eingabe-Events (Hotkeys) ----------
    fn on_input(&self, kid: String, pressed: bool) {
        // Hotkey-Aufnahme: nächste Taste/Maustaste wird der neue Hotkey
        if let Some(feat) = Feature::from_code(self.capturing.load(SeqCst)) {
            if pressed {
                lock(&self.hotkeys)[feat as usize] = kid;
                lock(&self.pressed).clear();
                self.capturing.store(0, SeqCst);
                self.wake();
            }
            return;
        }

        {
            let mut down = lock(&self.pressed);
            if pressed {
                if !down.insert(kid.clone()) {
                    return; // Tasten-Wiederholung ignorieren
                }
            } else {
                down.remove(&kid);
            }
        }

        let hotkeys = lock(&self.hotkeys).clone();
        let mode = self.cfg().mode;
        for feat in [Feature::Wardogs, Feature::Clicker, Feature::Bridge] {
            if kid != hotkeys[feat as usize] {
                continue;
            }
            if mode == Mode::Toggle && pressed {
                self.toggle(feat);
            } else if mode == Mode::Hold {
                if pressed {
                    if feat == Feature::Wardogs {
                        self.start_wardogs();
                    } else {
                        self.set_active(Some(feat));
                    }
                } else if self.active() == Some(feat) {
                    self.set_active(None);
                    if feat == Feature::Wardogs {
                        self.set_status("Abgebrochen");
                        self.cancel_countdown();
                    }
                }
            }
            self.wake();
        }
    }

    // ---------- Arbeits-Schleife ----------
    fn worker(self: Arc<Self>) {
        loop {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.step()));
            let error = match result {
                Ok(Ok(())) => continue,
                Ok(Err(e)) => e,
                Err(_) => "interner Fehler".to_string(),
            };
            // Der Thread darf nie sterben, sonst reagiert nichts mehr
            self.set_active(None);
            *lock(&self.deadline) = None;
            self.set_status(format!("Fehler: {error}"));
            eprintln!("Fehler in der Arbeits-Schleife: {error}");
            thread::sleep(ms(200));
        }
    }

    fn step(&self) -> Result<(), String> {
        let act = if self.is_capturing() { None } else { self.active() };
        if act != Some(Feature::Bridge) && (self.s_down.load(SeqCst) || self.sneak_down.load(SeqCst)) {
            self.release_bridge_keys();
        }

        match act {
            Some(Feature::Clicker) => {
                let cfg = self.cfg();
                input::click(cfg.button);
                self.clicks.fetch_add(1, SeqCst);
                self.wait(cfg.interval_ms.max(10), Feature::Clicker);
            }
            Some(Feature::Bridge) => self.bridge_cycle(),
            Some(Feature::Wardogs) => {
                let deadline = *lock(&self.deadline);
                match deadline {
                    None => {
                        // Countdown wurde abgebrochen, während die Schleife schon lief
                        if self.active() == Some(Feature::Wardogs) {
                            self.set_active(None);
                        }
                    }
                    Some(d) => {
                        let now = Instant::now();
                        if d > now {
                            let remaining = d - now;
                            let secs = remaining.as_secs_f64().ceil().max(1.0) as u64;
                            self.set_status(format!("Nicht bewegen: {secs}"));
                            self.wait_wake(remaining.min(ms(100)));
                        } else {
                            *lock(&self.deadline) = None;
                            let name = self.cfg().team.name();
                            self.set_status(format!("Suche nach {name}"));
                            self.wardogs_cycle()?;
                        }
                    }
                }
            }
            None => self.idle_wait(),
        }
        Ok(())
    }

    // ---------- Fast Bridge ----------
    fn sneak(&self, down: bool, cfg: &Config) {
        if down {
            let scan = match cfg.sneak_key {
                SneakKey::Ctrl => SC_LCTRL,
                SneakKey::Shift => SC_LSHIFT,
            };
            self.sneak_scan.store(scan, SeqCst);
        }
        input::send_scancode(self.sneak_scan.load(SeqCst), down);
        self.sneak_down.store(down, SeqCst);
    }

    fn release_bridge_keys(&self) {
        if self.sneak_down.load(SeqCst) {
            input::send_scancode(self.sneak_scan.load(SeqCst), false);
            self.sneak_down.store(false, SeqCst);
        }
        if self.s_down.swap(false, SeqCst) {
            input::send_scancode(SC_S, false);
        }
        let side = self.side_down.swap(0, SeqCst);
        if side != 0 {
            input::send_scancode(side, false);
        }
    }

    fn bridge_cycle(&self) {
        let cfg = self.cfg();
        let first = !self.sneak_down.load(SeqCst) && !self.s_down.load(SeqCst);

        // 1) ZUERST sneaken, damit man an der Kante gesichert ist
        if !self.sneak_down.load(SeqCst) {
            self.sneak(true, &cfg);
            if !self.wait(if first { 60 } else { 20 }, Feature::Bridge) {
                return;
            }
        }
        // 2) Bewegungstasten (S, optional A/D für Diagonal) gedrückt halten
        if !self.s_down.load(SeqCst) {
            input::send_scancode(SC_S, true);
            self.s_down.store(true, SeqCst);
            let side = match cfg.side {
                Side::A => SC_A,
                Side::D => SC_D,
                Side::Off => 0,
            };
            if side != 0 {
                input::send_scancode(side, true);
                self.side_down.store(side, SeqCst);
            }
        }
        // 3) Während der ganzen Sneak-Phase immer wieder rechtsklicken
        let end = Instant::now() + ms(cfg.sneak_ms);
        loop {
            input::click(Button::Right);
            if !self.wait(25, Feature::Bridge) {
                return;
            }
            if Instant::now() >= end {
                break;
            }
        }
        self.blocks.fetch_add(1, SeqCst);
        // 4) Erst jetzt entsneaken und ein Stück auf den neuen Block zurückgehen
        self.sneak(false, &cfg);
        self.wait(cfg.walk_ms, Feature::Bridge);
    }

    // ---------- Wardogs ----------
    fn start_wardogs(&self) {
        {
            let mut overlay = lock(&self.overlay);
            if overlay.is_none() {
                match Overlay::new() {
                    Ok(o) => *overlay = Some(o),
                    Err(e) => {
                        self.set_active(None);
                        eprintln!("Wardogs-Countdown konnte nicht gestartet werden: {e}");
                        self.set_status(e);
                        return;
                    }
                }
            }
            if let Some(o) = overlay.as_ref() {
                o.show(COUNTDOWN_SECONDS);
            }
        }
        *lock(&self.deadline) = Some(Instant::now() + Duration::from_secs(COUNTDOWN_SECONDS));
        self.set_active(Some(Feature::Wardogs));
        self.set_status(format!("Nicht bewegen: {COUNTDOWN_SECONDS}"));
    }

    fn cancel_countdown(&self) {
        *lock(&self.deadline) = None;
        if let Some(o) = lock(&self.overlay).as_ref() {
            o.hide();
        }
    }

    /// Lädt (und cached) die Vorlage eines Teams.
    fn template(&self, team: Team) -> Result<Arc<Template>, String> {
        if let Some(t) = lock(&self.templates).get(&team) {
            return Ok(t.clone());
        }
        let tpl = Arc::new(load_template(team.png(), &team.file())?);
        lock(&self.templates).insert(team, tpl.clone());
        Ok(tpl)
    }

    fn fail(&self, msg: impl Into<String>) -> Result<(), String> {
        self.set_active(None);
        self.set_status(msg);
        Ok(())
    }

    fn wardogs_cycle(&self) -> Result<(), String> {
        let team = self.cfg().team;
        let tpl = match self.template(team) {
            Ok(t) => t,
            Err(e) => return self.fail(e),
        };

        // Vordergrundfenster prüfen
        let (left, top, width, height) = unsafe {
            let hwnd = win::GetForegroundWindow();

            // WICHTIG: Das eigene Autoklicker-Fenster darf nie als Spiel durchgehen,
            // sonst wird das Logo in der eigenen Team-Auswahl angeklickt.
            let mut pid = 0u32;
            win::GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == std::process::id() {
                return self.fail("Wardogs-Spiel in den Vordergrund holen (nicht den Autoklicker)");
            }

            let mut title = [0u16; 512];
            let n = win::GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
            let title = String::from_utf16_lossy(&title[..n.max(0) as usize]).to_lowercase();
            if !title.contains("wardogs") {
                return self.fail("Wardogs-Spiel in den Vordergrund holen");
            }

            let mut client = win::RECT::default();
            if win::GetClientRect(hwnd, &mut client) == 0 {
                return Err("Wardogs-Clientbereich konnte nicht ermittelt werden.".into());
            }
            let mut origin = win::POINT { x: client.left, y: client.top };
            if win::ClientToScreen(hwnd, &mut origin) == 0 {
                return Err("Wardogs-Bildschirmposition konnte nicht ermittelt werden.".into());
            }
            (origin.x, origin.y, client.right - client.left, client.bottom - client.top)
        };
        if width <= 0 || height <= 0 {
            return Err("Wardogs-Fenster hat keine gültige Größe.".into());
        }

        // Countdown-Rahmen sicher weg, bevor der Screenshot gemacht wird
        if let Some(o) = lock(&self.overlay).as_ref() {
            o.hide();
        }
        thread::sleep(ms(60));

        let frame = capture::grab(left, top, width, height)?;
        let found = matcher::find_best(&frame, &tpl, &MATCH_SCALES);

        let found = match found {
            Some(f) if f.score >= MATCH_THRESHOLD => f,
            Some(f) => {
                return self.fail(format!(
                    "Team nicht gefunden (beste Übereinstimmung {}%)",
                    (f.score.max(0.0) * 100.0).round() as i64
                ));
            }
            None => return self.fail("Team nicht gefunden – Fenster zu klein/Ansicht prüfen"),
        };
        if self.active() != Some(Feature::Wardogs) {
            return Ok(()); // zwischendurch abgebrochen
        }

        let x = left + (found.x + found.w / 2) as i32;
        let y = top + (found.y + found.h / 2) as i32;
        if unsafe { win::SetCursorPos(x, y) } == 0 {
            return Err("Mauszeiger konnte nicht zum Wardogs-Team bewegt werden.".into());
        }
        thread::sleep(ms(120));
        input::click(Button::Left);
        self.wardogs_clicks.fetch_add(1, SeqCst);
        self.set_status(format!("{} angeklickt", team.name()));
        self.set_active(None);
        self.wake();
        Ok(())
    }

    // ---------- API für die Oberfläche ----------
    pub fn state_json(&self) -> String {
        let cfg = self.cfg();
        let hk = lock(&self.hotkeys).clone();
        let active = if self.is_capturing() { None } else { self.active() };
        format!(
            "{{\"active\":{},\"hotkeys\":{{\"wardogs\":{},\"clicker\":{},\"bridge\":{}}},\"mode\":{},\
             \"clicks\":{},\"blocks\":{},\"team\":{},\"wardogs_clicks\":{},\"wardogs_status\":{},\
             \"capturing\":{},\"maximized\":{}}}",
            active.map_or("null".to_string(), |f| json_str(f.key())),
            json_str(&input::key_display(&hk[0])),
            json_str(&input::key_display(&hk[1])),
            json_str(&input::key_display(&hk[2])),
            json_str(if cfg.mode == Mode::Hold { "hold" } else { "toggle" }),
            self.clicks.load(SeqCst),
            self.blocks.load(SeqCst),
            json_str(cfg.team.key()),
            self.wardogs_clicks.load(SeqCst),
            json_str(&lock(&self.status)),
            self.is_capturing(),
            self.maximized.load(SeqCst),
        )
    }

    /// `lines`: "schlüssel=wert"
    pub fn set_config<'a>(&self, lines: impl Iterator<Item = &'a str>) {
        let mut cfg = lock(&self.cfg);
        let mut new_team = None;
        let mut new_mode = None;
        for line in lines {
            let Some((key, value)) = line.split_once('=') else { continue };
            match key {
                "interval" => {
                    if let Ok(v) = value.parse::<i64>() {
                        cfg.interval_ms = v.clamp(10, 60000) as u64;
                    }
                }
                "sneak" => {
                    if let Ok(v) = value.parse::<i64>() {
                        cfg.sneak_ms = v.clamp(10, 500) as u64;
                    }
                }
                "walk" => {
                    if let Ok(v) = value.parse::<i64>() {
                        cfg.walk_ms = v.clamp(10, 500) as u64;
                    }
                }
                "sneak_key" => match value {
                    "ctrl" => cfg.sneak_key = SneakKey::Ctrl,
                    "shift" => cfg.sneak_key = SneakKey::Shift,
                    _ => {}
                },
                "side" => match value {
                    "none" => cfg.side = Side::Off,
                    "a" => cfg.side = Side::A,
                    "d" => cfg.side = Side::D,
                    _ => {}
                },
                "button" => match value {
                    "left" => cfg.button = Button::Left,
                    "middle" => cfg.button = Button::Middle,
                    "right" => cfg.button = Button::Right,
                    _ => {}
                },
                "team" => {
                    if let Some(t) = Team::parse(value) {
                        if t != cfg.team {
                            new_team = Some(t);
                        }
                    }
                }
                "mode" => match value {
                    "toggle" if cfg.mode != Mode::Toggle => new_mode = Some(Mode::Toggle),
                    "hold" if cfg.mode != Mode::Hold => new_mode = Some(Mode::Hold),
                    _ => {}
                },
                _ => {}
            }
        }
        if let Some(t) = new_team {
            cfg.team = t;
        }
        if let Some(m) = new_mode {
            cfg.mode = m;
        }
        drop(cfg);

        if let Some(t) = new_team {
            if self.active() == Some(Feature::Wardogs) {
                self.set_active(None);
                self.cancel_countdown();
            }
            self.set_status(format!("Ausgewählt: {}", t.name()));
            self.wake();
        }
        if new_mode.is_some() {
            if self.active() == Some(Feature::Wardogs) {
                self.set_status("Abgebrochen");
                self.cancel_countdown();
            }
            self.set_active(None);
            self.wake();
        }
    }

    pub fn toggle(&self, feat: Feature) {
        let mode = self.cfg().mode;
        if mode == Mode::Toggle || feat == Feature::Wardogs {
            if self.active() == Some(feat) {
                self.set_active(None);
                if feat == Feature::Wardogs {
                    self.set_status("Abgebrochen");
                    self.cancel_countdown();
                }
            } else if feat == Feature::Wardogs {
                self.start_wardogs();
            } else {
                self.set_active(Some(feat));
            }
            self.wake();
        }
    }

    pub fn reset_clicks(&self, feat: Feature) {
        match feat {
            Feature::Wardogs => self.wardogs_clicks.store(0, SeqCst),
            Feature::Bridge => self.blocks.store(0, SeqCst),
            Feature::Clicker => self.clicks.store(0, SeqCst),
        }
    }

    /// Die nächste Taste/Maustaste wird zum Hotkey für `feat`.
    pub fn capture_hotkey(&self, feat: Feature) {
        self.capturing.store(feat.code(), SeqCst);
        if self.active() == Some(Feature::Wardogs) {
            self.set_status("Abgebrochen");
            self.cancel_countdown();
        }
        self.set_active(None);
        self.wake();
    }

    /// Beim Beenden: nichts darf hängen bleiben.
    pub fn shutdown(&self) {
        self.set_active(None);
        self.cancel_countdown();
        self.release_bridge_keys();
    }
}

// ---------------------------------------------------------------
//  Hilfsfunktionen
// ---------------------------------------------------------------
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// PNG → Vorlage (zugeschnitten auf den sichtbaren Bereich, mit Maske).
fn load_template(bytes: &[u8], filename: &str) -> Result<Template, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| format!("PNG ungültig: {filename}"))?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|_| format!("PNG ungültig: {filename}"))?;
    let (w, h) = (info.width as usize, info.height as usize);
    let data = &buf[..info.buffer_size()];
    if w == 0 || h == 0 {
        return Err(format!("PNG ungültig: {filename}"));
    }

    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        _ => return Err(format!("PNG ungültig: {filename}")),
    };
    if data.len() < w * h * channels {
        return Err(format!("PNG ungültig: {filename}"));
    }

    let mut bgr = vec![0u8; w * h * 3];
    let mut alpha = vec![255u8; w * h];
    for i in 0..w * h {
        let px = &data[i * channels..(i + 1) * channels];
        let (r, g, b, a) = match channels {
            4 => (px[0], px[1], px[2], px[3]),
            3 => (px[0], px[1], px[2], 255),
            2 => (px[0], px[0], px[0], px[1]),
            _ => (px[0], px[0], px[0], 255),
        };
        bgr[i * 3] = b;
        bgr[i * 3 + 1] = g;
        bgr[i * 3 + 2] = r;
        alpha[i] = a;
    }
    let has_alpha = channels == 4 || channels == 2;

    let (tw, th, tbgr, mask) = if has_alpha {
        // Auf den sichtbaren Bereich (Alpha >= 128) zuschneiden
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
        for y in 0..h {
            for x in 0..w {
                if alpha[y * w + x] >= 128 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        if x1 <= x0 || y1 <= y0 {
            return Err(format!("PNG ist vollständig transparent: {filename}"));
        }
        let (cw, ch) = (x1 - x0, y1 - y0);
        let mut cbgr = Vec::with_capacity(cw * ch * 3);
        let mut cmask = Vec::with_capacity(cw * ch);
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * w + x;
                cbgr.extend_from_slice(&bgr[i * 3..i * 3 + 3]);
                cmask.push(if alpha[i] >= 128 { 1 } else { 0 });
            }
        }
        (cw, ch, cbgr, cmask)
    } else {
        (w, h, bgr, vec![1u8; w * h])
    };

    // Kontrast-Check (Standardabweichung der sichtbaren Pixel)
    let (mut sum, mut sum2, mut count) = (0f64, 0f64, 0f64);
    for (i, m) in mask.iter().enumerate() {
        if *m != 0 {
            for c in 0..3 {
                let v = tbgr[i * 3 + c] as f64;
                sum += v;
                sum2 += v * v;
                count += 1.0;
            }
        }
    }
    if count > 0.0 {
        let mean = sum / count;
        let std = (sum2 / count - mean * mean).max(0.0).sqrt();
        if std < 4.0 {
            return Err(format!("PNG hat zu wenig Kontrast: {filename}"));
        }
    }

    Ok(Template { w: tw, h: th, bgr: tbgr, mask })
}
