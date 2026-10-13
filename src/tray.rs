//! Symbol im Infobereich der Taskleiste (nur Windows) und Befehle, die von
//! außerhalb des Fensters an die Oberfläche gehen.

use std::sync::mpsc::Sender;

use eframe::egui;

pub const SUPPORTED: bool = cfg!(windows);

/// Befehle aus dem Infobereich oder von einem erneuten Programmstart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Command {
    /// Fenster anzeigen und in den Vordergrund holen.
    Show,
    CheckUpdates,
    Quit,
}

/// Was das Symbol gerade anzeigt.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// Kurzer Status, z. B. „3 Updates verfügbar“.
    pub status: String,
    /// Punkt am Symbol: Updates verfügbar.
    pub badge: bool,
    /// Eine Update-Suche läuft oder wartet bereits.
    pub checking: bool,
}

/// Schickt einen Befehl an die Oberfläche und weckt sie auf – auch wenn das
/// Fenster gerade versteckt ist.
#[derive(Clone)]
pub struct CommandSender {
    commands: Sender<Command>,
    ctx: egui::Context,
}

impl CommandSender {
    pub fn new(commands: Sender<Command>, ctx: egui::Context) -> Self {
        Self { commands, ctx }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
        self.ctx.request_repaint();
    }
}

pub use imp::Tray;

#[cfg(windows)]
mod imp {
    use std::sync::Once;

    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    use super::{Command, CommandSender, State};

    const SHOW: &str = "show";
    const CHECK: &str = "check";
    const QUIT: &str = "quit";

    pub struct Tray {
        icon: TrayIcon,
        status: MenuItem,
        check: MenuItem,
        state: State,
    }

    impl Tray {
        pub fn new(commands: &CommandSender) -> Result<Self, String> {
            install_handlers(commands);
            let status = MenuItem::new("WPM", false, None);
            let check = MenuItem::with_id(CHECK, "Nach Updates suchen", true, None);
            let menu = Menu::with_items(&[
                &status,
                &PredefinedMenuItem::separator(),
                &MenuItem::with_id(SHOW, "WPM öffnen", true, None),
                &check,
                &PredefinedMenuItem::separator(),
                &MenuItem::with_id(QUIT, "Beenden", true, None),
            ])
            .map_err(|e| e.to_string())?;
            let icon = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                // Linksklick öffnet das Fenster, Rechtsklick zeigt das Menü.
                .with_menu_on_left_click(false)
                .with_tooltip("WPM – Windows-Paketmanager")
                .with_icon(icon(false)?)
                .build()
                .map_err(|e| e.to_string())?;
            Ok(Self {
                icon,
                status,
                check,
                state: State::default(),
            })
        }

        /// Übernimmt einen neuen Zustand; ruft Windows nur bei Änderungen auf.
        pub fn update(&mut self, state: State) {
            if state == self.state {
                return;
            }
            if state.status != self.state.status {
                self.status.set_text(&state.status);
                // Windows zeigt höchstens 127 Zeichen als Tooltip.
                let tooltip: String = format!("WPM – {}", state.status)
                    .chars()
                    .take(120)
                    .collect();
                let _ = self.icon.set_tooltip(Some(tooltip));
            }
            if state.badge != self.state.badge
                && let Ok(icon) = icon(state.badge)
            {
                let _ = self.icon.set_icon(Some(icon));
            }
            self.check.set_enabled(!state.checking);
            self.state = state;
        }
    }

    fn icon(badge: bool) -> Result<Icon, String> {
        let (rgba, size) = crate::icon::tray_icon(badge);
        Icon::from_rgba(rgba, size, size).map_err(|e| e.to_string())
    }

    /// Klicks aufs Symbol und Menüeinträge werden zu [`Command`]s. Die Handler
    /// lassen sich nur einmal pro Prozess setzen und gelten auch für ein
    /// später neu angelegtes Symbol.
    fn install_handlers(commands: &CommandSender) {
        static INSTALLED: Once = Once::new();
        INSTALLED.call_once(|| {
            let sender = commands.clone();
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                let open = matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } | TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                );
                if open {
                    sender.send(Command::Show);
                }
            }));
            let sender = commands.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let command = if event.id == SHOW {
                    Command::Show
                } else if event.id == CHECK {
                    Command::CheckUpdates
                } else if event.id == QUIT {
                    Command::Quit
                } else {
                    return;
                };
                sender.send(command);
            }));
        });
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{CommandSender, State};

    pub struct Tray;

    impl Tray {
        pub fn new(_commands: &CommandSender) -> Result<Self, String> {
            Err("nur unter Windows verfügbar".into())
        }

        pub fn update(&mut self, _state: State) {}
    }
}
