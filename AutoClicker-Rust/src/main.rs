#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("Der Wardogs Autoklicker läuft nur unter Windows.");

mod capture;
mod engine;
mod input;
mod matcher;
mod overlay;
mod ui;
mod win;

use engine::{Engine, Feature};
use std::thread;
use std::time::Duration;
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{ResizeDirection, WindowBuilder};
use wry::WebViewBuilder;

/// Nachrichten an den UI-Thread (Fenster und WebView dürfen nur dort angefasst werden).
enum UserEvent {
    /// Aktuellen Zustand sofort an die Oberfläche schicken
    Push,
    /// Bereits berechneter Zustand (vom Hintergrund-Thread)
    State(String),
    /// JavaScript ausführen
    Eval(String),
    Drag,
    Resize(ResizeDirection),
    Minimize,
    ToggleMaximize,
    Close,
}

fn main() {
    if let Err(e) = run() {
        win::message_box_error("Wardogs Autoklicker", &e);
    }
}

fn run() -> Result<(), String> {
    unsafe {
        // Screenshot- und Mauskoordinaten müssen bei 125 % / 150 % Skalierung übereinstimmen
        win::SetProcessDpiAwarenessContext(-4); // PER_MONITOR_AWARE_V2
        // 1-ms-Timer: sorgt für genaue Klick-Intervalle (Standard wäre ~15 ms)
        win::timeBeginPeriod(1);
    }

    let engine = Engine::new();
    engine.start();

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("WARDOGS AUTOKLICKER")
        .with_inner_size(LogicalSize::new(1040.0, 700.0))
        .with_min_inner_size(LogicalSize::new(860.0, 600.0))
        .with_decorations(false) // eigene Titelleiste aus index.html
        .build(&event_loop)
        .map_err(|e| format!("Fenster konnte nicht erstellt werden: {e}"))?;

    let ipc_engine = engine.clone();
    let ipc_proxy = proxy.clone();
    let webview = WebViewBuilder::new()
        .with_html(ui::build_html())
        .with_background_color((6, 9, 8, 255))
        .with_ipc_handler(move |request| handle_ipc(request.body(), &ipc_engine, &ipc_proxy))
        .build(&window)
        .map_err(|e| {
            format!(
                "Die Oberfläche (WebView2) konnte nicht gestartet werden:\n{e}\n\n\
                 Bitte die „Microsoft Edge WebView2 Runtime“ installieren:\n\
                 https://developer.microsoft.com/microsoft-edge/webview2/"
            )
        })?;

    // Zustand nur bei Änderung an die Oberfläche schicken (kein Dauer-Polling vom JS aus)
    {
        let engine = engine.clone();
        let proxy = proxy.clone();
        thread::Builder::new()
            .name("ui-push".into())
            .spawn(move || {
                let mut last = String::new();
                loop {
                    thread::sleep(Duration::from_millis(100));
                    let json = engine.state_json();
                    if json != last {
                        last = json.clone();
                        if proxy.send_event(UserEvent::State(json)).is_err() {
                            break;
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
    }

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Push) => {
                let _ = webview.evaluate_script(&format!("window.__state({})", engine.state_json()));
            }
            Event::UserEvent(UserEvent::State(json)) => {
                let _ = webview.evaluate_script(&format!("window.__state({json})"));
            }
            Event::UserEvent(UserEvent::Eval(js)) => {
                let _ = webview.evaluate_script(&js);
            }
            Event::UserEvent(UserEvent::Drag) => {
                let _ = window.drag_window();
            }
            Event::UserEvent(UserEvent::Resize(direction)) => {
                let _ = window.drag_resize_window(direction);
            }
            Event::UserEvent(UserEvent::Minimize) => window.set_minimized(true),
            Event::UserEvent(UserEvent::ToggleMaximize) => {
                let maximize = !window.is_maximized();
                window.set_maximized(maximize);
                engine.set_maximized(maximize);
            }
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => {
                engine.set_maximized(window.is_maximized());
            }
            Event::UserEvent(UserEvent::Close)
            | Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                // Gedrückte Tasten (Fast Bridge) loslassen, dann komplett beenden
                // (beendet auch die Hook-Threads)
                engine.shutdown();
                std::process::exit(0);
            }
            _ => {}
        }
    })
}

/// Befehle aus dem JavaScript: Zeilen getrennt durch '\n', erste Zeile = Befehl.
fn handle_ipc(message: &str, engine: &Engine, proxy: &EventLoopProxy<UserEvent>) {
    let send = |event: UserEvent| {
        let _ = proxy.send_event(event);
    };
    let mut parts = message.split('\n');
    match parts.next().unwrap_or("") {
        "ready" => {
            send(UserEvent::Push);
            if let Some(script) = ui::custom_background_script() {
                send(UserEvent::Eval(script));
            }
        }
        "set_config" => engine.set_config(parts),
        "toggle" => {
            if let Some(feat) = parts.next().and_then(Feature::parse) {
                engine.toggle(feat);
                send(UserEvent::Push);
            }
        }
        "reset" => {
            if let Some(feat) = parts.next().and_then(Feature::parse) {
                engine.reset_clicks(feat);
                send(UserEvent::Push);
            }
        }
        "capture" => {
            if let Some(feat) = parts.next().and_then(Feature::parse) {
                engine.capture_hotkey(feat);
                send(UserEvent::Push);
            }
        }
        "drag" => send(UserEvent::Drag),
        "resize" => {
            let direction = match parts.next().unwrap_or("") {
                "n" => Some(ResizeDirection::North),
                "s" => Some(ResizeDirection::South),
                "e" => Some(ResizeDirection::East),
                "w" => Some(ResizeDirection::West),
                "ne" => Some(ResizeDirection::NorthEast),
                "nw" => Some(ResizeDirection::NorthWest),
                "se" => Some(ResizeDirection::SouthEast),
                "sw" => Some(ResizeDirection::SouthWest),
                _ => None,
            };
            if let Some(d) = direction {
                send(UserEvent::Resize(d));
            }
        }
        "minimize" => send(UserEvent::Minimize),
        "maximize" => send(UserEvent::ToggleMaximize),
        "close" => send(UserEvent::Close),
        _ => {}
    }
}
