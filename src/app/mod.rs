//! Zustand und Logik der Oberfläche. Die einzelnen Reiter liegen in eigenen Dateien.

mod details;
mod installed;
mod licenses;
mod log;
mod search;
mod settings_tab;
mod updates;
mod widgets;

use std::collections::{HashMap, HashSet, VecDeque};
use std::mem::discriminant;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};

use crate::i18n;
use crate::runner::{Runner, WingetRunner};
use crate::scheduler;
use crate::self_update;
use crate::settings::Settings;
use crate::system;
use crate::tray::{self, Command, CommandSender, Tray};
use crate::winget::demo::DemoRunner;
use crate::winget::parse;
use crate::winget::{
    Details, Failure, Options, Package, Pin, Request, Response, Source, UpgradeList,
};
use crate::worker::{JobId, Worker, WorkerEvent};

const MAX_LOG_LINES: usize = 5000;
/// Ab so langer Stille eines Installers weist die Statusleiste darauf hin.
const SILENCE_HINT: Duration = Duration::from_secs(180);

pub struct StartupFlags {
    pub demo: bool,
    /// Vom Autostart gestartet bzw. ausdrücklich minimiert gewünscht. Mit
    /// Symbol im Infobereich startet WPM dann ganz ohne sichtbares Fenster.
    pub minimized: bool,
    /// Nach dem Schließen der Instanz mit Adminrechten im Hintergrund neu
    /// gestartet: bekannte Updates nicht erneut melden.
    pub background: bool,
    /// WPM läuft mit Administratorrechten.
    pub elevated: bool,
    /// Läuft mit Adminrechten eines anderen Benutzerkontos.
    pub other_account: bool,
    /// Nach einem Selbst-Update neu gestartet: Ergebnis (Fehler als Kurzfassung).
    pub self_update: Option<Result<(), String>>,
    /// Paket-IDs, deren Update nach dem Neustart mit Adminrechten weiterläuft
    /// (`*` = alle nicht angehefteten).
    pub resume_upgrades: Vec<String>,
}

/// Steht in `resume_upgrades` für „alle verfügbaren Updates“.
pub const ALL_UPGRADES: &str = "*";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Updates,
    Installed,
    Search,
    Settings,
    Log,
}

/// Bearbeitungsstand eines Pakets (für die Spalte „Aktionen“).
enum PackageStatus {
    Queued,
    Running,
    /// Mit Hinweis von winget; ohne wird „Erledigt“ in der aktuellen Sprache angezeigt.
    Done(Option<String>),
    Failed(Failure),
}

struct QueuedJob {
    id: JobId,
    request: Request,
    running: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NoticeKind {
    Info,
    Success,
    Error,
}

struct Notice {
    kind: NoticeKind,
    title: String,
    /// Aufklappbare Zusatzinformationen (z. B. Ausgabe von winget).
    details: String,
    expanded: bool,
    /// Knopf „Als Administrator neu starten“ anbieten.
    admin_hint: bool,
    /// Fehlgeschlagenes Update, das nach dem Neustart erneut versucht wird.
    retry: Option<Package>,
    /// Erklärung, was man tun kann (immer sichtbar, unter dem Titel).
    advice: String,
    /// Protokolldatei des Installers.
    log_path: Option<std::path::PathBuf>,
    /// Paket, dessen Anheften angeboten wird.
    pin: Option<Package>,
    /// Bleibt stehen, bis sie geschlossen wird (Anleitungen).
    persistent: bool,
    created: Instant,
}

impl Notice {
    fn new(kind: NoticeKind, title: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            details: String::new(),
            expanded: false,
            admin_hint: false,
            retry: None,
            advice: String::new(),
            log_path: None,
            pin: None,
            persistent: false,
            created: Instant::now(),
        }
    }
}

struct DetailsView {
    package: Package,
    result: Option<Result<Details, Failure>>,
}

/// Benutzeraktionen aus den Tabellen; werden nach dem Zeichnen ausgeführt.
enum Action {
    CheckUpdates,
    LoadInstalled,
    Install(Package),
    Upgrade(Package),
    Uninstall(Package),
    Pin(Package),
    Unpin(String),
    ShowDetails(Package),
}

pub struct WpmApp {
    tab: Tab,
    settings: Settings,
    shared_settings: Arc<RwLock<Settings>>,
    worker: Worker,
    queue: Vec<QueuedJob>,
    /// Letzter `\r`-Zwischenstand von winget (nur im Demo-Modus bzw. wenn winget
    /// einmal Fortschritt in eine Pipe schreibt).
    progress: String,
    /// Letzte Statuszeile des laufenden Auftrags, z. B. „Wird heruntergeladen …“.
    last_line: String,
    job_started: Option<Instant>,
    /// Zeitpunkt der letzten Ausgabe des laufenden Auftrags.
    last_output: Option<Instant>,

    installed: Option<Vec<Package>>,
    installed_filter: String,
    updates: Option<UpgradeList>,
    updates_checked_at: Option<Instant>,
    selected: HashSet<String>,
    search_query: String,
    search_results: Option<Vec<Package>>,
    status: HashMap<String, PackageStatus>,

    details: Option<DetailsView>,
    confirm_uninstall: Option<Package>,
    /// Fenster „Lizenzen“ ist offen.
    show_licenses: bool,
    pins: Option<Vec<Pin>>,
    sources: Option<Vec<Source>>,
    winget_version: Option<Result<String, String>>,
    log: VecDeque<String>,
    notices: Vec<Notice>,

    needs_refresh: bool,
    focus_search: bool,
    elevated: bool,
    /// Erfolgreich aktualisierte Pakete (ID → Version) bis zur nächsten Prüfung.
    recently_upgraded: HashMap<String, String>,
    /// Updates, für die WPM gleich mit Adminrechten neu startet.
    elevate_upgrades: Vec<Package>,
    /// Nach einem Neustart mit Adminrechten fortzusetzende Updates (IDs).
    resume_upgrades: Vec<String>,
    /// HWND des Hauptfensters (für die Windows-Bestätigung im Vordergrund).
    window: Option<isize>,
    start_minimized: bool,
    first_frame: bool,
    demo: bool,
    window_title: String,

    /// Symbol im Infobereich (nur Windows und nur, wenn eingeschaltet).
    tray: Option<Tray>,
    /// Befehle vom Symbol im Infobereich und von einem erneuten Programmstart.
    commands: Receiver<Command>,
    command_sender: CommandSender,
    /// Fenster ist versteckt, WPM läuft im Infobereich.
    hidden: bool,
    /// WPM wird beendet; das Schließen nicht mehr in den Infobereich umleiten.
    quitting: bool,
    /// Mit Adminrechten geschlossen: sobald nichts mehr läuft, ohne
    /// Adminrechte im Infobereich neu starten.
    leave_elevated: bool,
    /// Ergebnisse von Installationen o. Ä. bei verstecktem Fenster, für eine
    /// Benachrichtigung, sobald alles erledigt ist.
    background_results: Vec<(String, bool)>,
    /// Ergebnis der nächsten Update-Suche melden (aus dem Infobereich gestartet).
    announce_check: bool,
    /// Update von WPM selbst, sobald alle anderen Aufträge durch sind.
    self_update: Option<Package>,
    /// Ein anderes Update schlug fehl, während das von WPM wartete.
    self_update_blocked: bool,
}

impl WpmApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        mut settings: Settings,
        flags: StartupFlags,
    ) -> Self {
        let ctx = &cc.egui_ctx;
        setup_style(ctx);
        ctx.set_theme(settings.theme.to_egui());

        // Maßgeblich ist, was tatsächlich eingetragen ist. Ein vorhandener
        // Eintrag wird mit dem aktuellen Pfad erneuert, falls die EXE verschoben
        // wurde (nicht unter einem fremden Konto – das wäre dessen Autostart).
        if system::AUTOSTART_SUPPORTED {
            settings.autostart = system::autostart_enabled();
            if settings.autostart && !flags.other_account {
                let _ = system::set_autostart(true);
            }
        }

        // Startmenü: genauso. winget legt für portable Programme keinen
        // Eintrag an; über winget installiert legt WPM ihn deshalb beim ersten
        // normalen Start selbst an (danach nie wieder ungefragt).
        if system::START_MENU_SUPPORTED {
            settings.start_menu = system::start_menu_entry_exists();
            if !flags.demo && !flags.other_account {
                if settings.start_menu {
                    let _ = system::refresh_start_menu_entry();
                }
                if !settings.start_menu_offered && !flags.elevated && system::installed_by_winget()
                {
                    if !settings.start_menu {
                        settings.start_menu = system::set_start_menu_entry(true).is_ok();
                    }
                    // Schlug das Anlegen fehl (z. B. umgeleiteter Ordner noch
                    // nicht erreichbar), beim nächsten Start erneut versuchen.
                    if settings.start_menu {
                        settings.start_menu_offered = true;
                        let _ = settings.save();
                    }
                }
            }
        }

        let runner: Arc<dyn Runner> = if flags.demo {
            Arc::new(DemoRunner::default())
        } else {
            Arc::new(WingetRunner::default())
        };
        let worker = Worker::spawn(runner, ctx.clone());
        let (sender, commands) = mpsc::channel();
        let command_sender = CommandSender::new(sender, ctx.clone());
        if !flags.demo {
            // Ein zweiter Start von WPM holt das Fenster nach vorn.
            let sender = command_sender.clone();
            system::listen_for_activation(move || sender.send(Command::Show));
        }
        let shared_settings = Arc::new(RwLock::new(settings.clone()));
        scheduler::spawn(
            shared_settings.clone(),
            worker.sender.clone(),
            worker.last_check.clone(),
        );

        let mut app = Self {
            tab: Tab::Updates,
            settings,
            shared_settings,
            worker,
            queue: Vec::new(),
            progress: String::new(),
            last_line: String::new(),
            job_started: None,
            last_output: None,
            installed: None,
            installed_filter: String::new(),
            updates: None,
            updates_checked_at: None,
            selected: HashSet::new(),
            search_query: String::new(),
            search_results: None,
            status: HashMap::new(),
            details: None,
            confirm_uninstall: None,
            show_licenses: false,
            pins: None,
            sources: None,
            winget_version: None,
            log: VecDeque::new(),
            notices: Vec::new(),
            needs_refresh: false,
            focus_search: false,
            elevated: flags.elevated,
            recently_upgraded: HashMap::new(),
            elevate_upgrades: Vec::new(),
            resume_upgrades: flags.resume_upgrades,
            window: system::native_window(cc),
            start_minimized: flags.minimized,
            first_frame: true,
            demo: flags.demo,
            window_title: String::new(),
            tray: None,
            commands,
            command_sender,
            hidden: false,
            quitting: false,
            leave_elevated: false,
            background_results: Vec::new(),
            announce_check: false,
            self_update: None,
            self_update_blocked: false,
        };
        app.sync_tray();

        match flags.self_update {
            Some(Ok(())) => {
                let text = trf!(
                    "WPM wurde auf Version {} aktualisiert.",
                    "WPM was updated to version {}.",
                    env!("CARGO_PKG_VERSION")
                );
                if flags.minimized {
                    system::notify(tr!("WPM aktualisiert", "WPM updated"), &text);
                }
                app.notice(NoticeKind::Success, text);
            }
            Some(Err(summary)) => {
                if flags.minimized {
                    system::notify(
                        tr!("Update von WPM fehlgeschlagen", "WPM update failed"),
                        &summary,
                    );
                }
                app.notices.push(Notice {
                    advice: trf!(
                        "WPM beenden (Rechtsklick auf das Symbol im Infobereich, „Beenden“) und in einem Terminal \
                         „winget upgrade {}“ ausführen.",
                        "Quit WPM (right-click the icon in the system tray, “Quit”) and run \
                         “winget upgrade {}” in a terminal.",
                        self_update::PACKAGE_ID
                    ),
                    ..Notice::new(
                        NoticeKind::Error,
                        trf!(
                            "Update von WPM fehlgeschlagen: {summary}",
                            "WPM update failed: {summary}"
                        ),
                    )
                });
            }
            None => {}
        }
        if flags.other_account {
            app.notice(
                NoticeKind::Error,
                tr!(
                    "WPM läuft nach der Windows-Bestätigung unter einem anderen Benutzerkonto \
                     (andere Zugangsdaten eingegeben oder Windows-Administratorschutz aktiv). \
                     Benutzerbezogene Programme und Einstellungen gehören dann zu diesem Konto.",
                    "After the UAC prompt, WPM is running under a different user account \
                     (other credentials were entered or Windows Administrator Protection is active). \
                     Per-user programs and settings then belong to that account."
                ),
            );
        }
        app.submit(Request::Version);
        // Nach einem Neustart mit Adminrechten zuerst die Liste holen, dann
        // die gewünschten Updates fortsetzen (siehe `resume_after_restart`).
        if app.settings.check_on_startup || !app.resume_upgrades.is_empty() {
            let notify = flags.minimized && !flags.background && app.settings.notify;
            app.worker
                .sender
                .submit(app.check_request(), app.options(), notify);
        }
        app
    }

    fn options(&self) -> Options {
        Options {
            mode: self.settings.install_mode,
            scope: self.settings.scope,
        }
    }

    fn check_request(&self) -> Request {
        Request::CheckUpdates {
            include_unknown: self.settings.include_unknown,
        }
    }

    /// Reiht einen Auftrag ein, sofern nicht schon ein gleichartiger wartet.
    fn submit(&mut self, request: Request) {
        let duplicate = self.queue.iter().any(|job| {
            discriminant(&job.request) == discriminant(&request)
                && job.request.package_id() == request.package_id()
                && !matches!(request, Request::Search { .. } | Request::Show { .. })
        });
        if !duplicate {
            self.worker.sender.submit(request, self.options(), false);
        }
    }

    fn busy(&self, predicate: impl Fn(&Request) -> bool) -> bool {
        self.queue.iter().any(|job| predicate(&job.request))
    }

    fn notice(&mut self, kind: NoticeKind, title: impl Into<String>) {
        self.notices.push(Notice::new(kind, title));
    }

    fn notice_failure(&mut self, request: &Request, failure: &Failure) {
        let upgrade = match request {
            Request::Upgrade { package } => Some(package.clone()),
            _ => None,
        };
        self.notices.push(Notice {
            details: failure.details.clone(),
            admin_hint: failure.needs_admin,
            retry: upgrade.clone().filter(|_| failure.needs_admin),
            advice: failure.advice.unwrap_or_default().to_string(),
            log_path: failure.log_path.clone(),
            pin: upgrade.filter(|_| failure.suggest_pin),
            ..Notice::new(
                NoticeKind::Error,
                trf!(
                    "{} fehlgeschlagen: {}",
                    "{} failed: {}",
                    request.label(),
                    failure.summary
                ),
            )
        });
    }

    /// Updates mit Adminrechten installieren, aber WPM läuft noch ohne?
    fn wants_elevation(&self) -> bool {
        self.settings.run_as_admin && system::ELEVATION_SUPPORTED && !self.elevated
    }

    /// Ein Neustart bricht laufende Abfragen ab, aber nie eine Installation.
    fn can_restart_elevated(&self) -> bool {
        system::ELEVATION_SUPPORTED
            && !self.elevated
            && !self.queue.iter().any(|job| job.request.changes_packages())
    }

    /// Startet WPM mit Administratorrechten neu (Windows fragt einmal nach);
    /// die neue Instanz setzt die Updates von `upgrade_ids` fort. Gibt bei
    /// abgelehnter Bestätigung `false` zurück.
    fn restart_elevated(&mut self, ctx: &egui::Context, upgrade_ids: &[String]) -> bool {
        let args = crate::relaunch_args(self.demo, upgrade_ids);
        match system::relaunch_elevated(&args, self.window) {
            Ok(()) => {
                self.worker.cancel_all();
                self.quit(ctx);
                true
            }
            Err(e) => {
                self.notice(
                    NoticeKind::Info,
                    trf!(
                        "Kein Neustart mit Administratorrechten: {e}",
                        "Could not restart as administrator: {e}"
                    ),
                );
                false
            }
        }
    }

    /// Gesammelte Updates mit einem Neustart mit Adminrechten fortsetzen –
    /// sobald keine Installation mehr läuft. Wird die Bestätigung abgelehnt,
    /// laufen die Updates ohne Adminrechte (Installer fragen dann einzeln).
    fn start_elevated_upgrades(&mut self, ctx: &egui::Context) {
        if self.elevate_upgrades.is_empty() {
            return;
        }
        if !self.wants_elevation() {
            // Einstellung inzwischen ausgeschaltet: normal installieren.
            for package in std::mem::take(&mut self.elevate_upgrades) {
                self.status.remove(&package.id);
                self.submit(Request::Upgrade { package });
            }
            return;
        }
        if !self.can_restart_elevated() {
            return;
        }
        if self.hidden {
            // Die Windows-Bestätigung braucht ein sichtbares Fenster, sonst
            // blinkt sie nur in der Taskleiste und blockiert WPM.
            self.show_window(ctx);
            return;
        }
        let packages = std::mem::take(&mut self.elevate_upgrades);
        let all_available: HashSet<&str> = self
            .updates
            .iter()
            .flat_map(|u| &u.upgrades)
            .map(|p| p.id.as_str())
            .collect();
        // Ein vorgemerktes Update von WPM selbst setzt die neue Instanz fort.
        let requested: HashSet<&str> = packages
            .iter()
            .chain(&self.self_update)
            .map(|p| p.id.as_str())
            .collect();
        let ids = if !all_available.is_empty() && requested == all_available {
            vec![ALL_UPGRADES.to_string()]
        } else {
            requested.iter().map(|id| id.to_string()).collect()
        };
        if !self.restart_elevated(ctx, &ids) {
            for package in packages {
                self.status.remove(&package.id);
                self.submit(Request::Upgrade { package });
            }
        }
    }

    /// Nach einem Neustart mit Adminrechten: die gewünschten Updates aus der
    /// frisch geholten Liste starten. Es werden nur Pakete aktualisiert, für
    /// die winget selbst ein Update meldet.
    fn resume_after_restart(&mut self, list: &UpgradeList) {
        let ids = std::mem::take(&mut self.resume_upgrades);
        if ids.is_empty() {
            return;
        }
        let packages: Vec<Package> = if ids.iter().any(|id| id == ALL_UPGRADES) {
            list.upgrades.clone()
        } else {
            let found: Vec<Package> = list
                .upgrades
                .iter()
                .chain(&list.pinned)
                .filter(|p| ids.contains(&p.id))
                .cloned()
                .collect();
            let missing: Vec<&str> = ids
                .iter()
                .filter(|id| !found.iter().any(|p| &p.id == *id))
                .map(String::as_str)
                .collect();
            if !missing.is_empty() {
                self.notice(
                    NoticeKind::Info,
                    trf!(
                        "Kein Update mehr gefunden für: {}",
                        "Update no longer available for: {}",
                        missing.join(", ")
                    ),
                );
            }
            found
        };
        for package in packages {
            self.apply(Action::Upgrade(package));
        }
    }

    /// Merkt das Update von WPM selbst vor: Es läuft erst, wenn alle anderen
    /// Aufträge durch sind, weil sich WPM dafür beendet.
    fn queue_self_update(&mut self, package: Package) {
        self.status
            .insert(package.id.clone(), PackageStatus::Queued);
        self.self_update = Some(package);
        self.self_update_blocked = false;
    }

    /// Startet das vorgemerkte Selbst-Update (siehe `self_update`).
    fn start_self_update(&mut self, ctx: &egui::Context) {
        let busy = !self.elevate_upgrades.is_empty()
            || self.queue.iter().any(|job| job.request.changes_packages());
        if self.self_update.is_none() || busy || self.quitting {
            return;
        }
        let Some(package) = self.self_update.take() else {
            return;
        };
        // Versteckt meldet notify_background_results die Fehler ohnehin.
        if std::mem::take(&mut self.self_update_blocked) && !self.hidden {
            self.status.remove(&package.id);
            self.notice(
                NoticeKind::Info,
                tr!(
                    "Das Update von WPM wurde zurückgestellt, weil andere Updates fehlgeschlagen \
                     sind. Nach dem Prüfen der Meldungen WPM erneut aktualisieren.",
                    "The WPM update was postponed because other updates failed. \
                     After reviewing the messages, update WPM again."
                ),
            );
            return;
        }
        if self.demo {
            self.status.remove(&package.id);
            self.notice(
                NoticeKind::Info,
                tr!(
                    "Demo-Modus: Hier würde sich WPM beenden, über winget aktualisieren und \
                     danach von selbst neu starten.",
                    "Demo mode: WPM would now quit, update itself via winget and \
                     then restart automatically."
                ),
            );
            return;
        }
        // Systemweit installiert (unter „Programme“): Das Update braucht Adminrechte.
        let needs_admin = system::exe_in_protected_folder();
        if needs_admin && !self.elevated && system::ELEVATION_SUPPORTED {
            if self.hidden {
                // Die Windows-Bestätigung braucht ein sichtbares Fenster.
                self.self_update = Some(package);
                self.show_window(ctx);
                return;
            }
            if !self.restart_elevated(ctx, &[self_update::PACKAGE_ID.to_string()]) {
                self.status.remove(&package.id);
                self.notice(
                    NoticeKind::Error,
                    tr!(
                        "WPM ist systemweit installiert – für das Update sind Administratorrechte nötig.",
                        "WPM is installed system-wide – the update requires administrator rights."
                    ),
                );
            }
            return;
        }
        let launch = self_update::Launch {
            hidden: self.hidden,
            drop_elevation: self.elevated && !needs_admin,
        };
        match self_update::start(&launch) {
            Ok(()) => {
                if !self.hidden {
                    system::notify(
                        tr!("WPM wird aktualisiert", "Updating WPM"),
                        tr!(
                            "WPM startet danach von selbst neu – bitte nicht vorher erneut starten.",
                            "WPM will restart automatically afterwards – please do not start it again before then."
                        ),
                    );
                }
                self.quit(ctx);
            }
            Err(e) => {
                self.status.remove(&package.id);
                self.notice(
                    NoticeKind::Error,
                    trf!(
                        "Update von WPM konnte nicht gestartet werden: {e}",
                        "Could not start the WPM update: {e}"
                    ),
                );
            }
        }
    }

    fn notice_self_uninstall(&mut self) {
        if self.notices.iter().any(|n| n.persistent) {
            return;
        }
        self.notices.push(Notice {
            advice: trf!(
                "„Mit Windows starten“ und „Im Startmenü anzeigen“ ausschalten, WPM beenden \
                 (Rechtsklick auf das Symbol im Infobereich, „Beenden“) und dann in einem Terminal \
                 „winget uninstall {}“ ausführen – oder WPM unter Windows-Einstellungen > Apps \
                 deinstallieren.",
                "Turn off “Start with Windows” and “Show in Start menu”, quit WPM (right-click the \
                 icon in the system tray, “Quit”) and then run “winget uninstall {}” in a terminal – \
                 or uninstall WPM under Windows Settings > Apps.",
                self_update::PACKAGE_ID
            ),
            persistent: true,
            ..Notice::new(
                NoticeKind::Info,
                tr!(
                    "WPM kann sich nicht selbst deinstallieren, solange es läuft.",
                    "WPM cannot uninstall itself while it is running."
                ),
            )
        });
    }

    fn installed_ids(&self) -> HashSet<&str> {
        self.installed
            .iter()
            .flatten()
            .map(|p| p.id.as_str())
            .collect()
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::CheckUpdates => self.submit(self.check_request()),
            Action::LoadInstalled => self.submit(Request::ListInstalled),
            Action::Install(package) => self.submit(Request::Install { package }),
            Action::Upgrade(package) if self_update::is_self(&package) => {
                self.queue_self_update(package);
            }
            Action::Upgrade(package) if self.wants_elevation() => {
                // Erst sammeln: alle Updates dieses Klicks laufen nach einem
                // einzigen Neustart mit Adminrechten.
                self.status
                    .insert(package.id.clone(), PackageStatus::Queued);
                if !self.elevate_upgrades.iter().any(|p| p.id == package.id) {
                    self.elevate_upgrades.push(package);
                }
            }
            Action::Upgrade(package) => self.submit(Request::Upgrade { package }),
            Action::Uninstall(package) if self_update::is_self(&package) => {
                self.notice_self_uninstall();
            }
            Action::Uninstall(package) => {
                if self.settings.confirm_uninstall {
                    self.confirm_uninstall = Some(package);
                } else {
                    self.submit(Request::Uninstall { package });
                }
            }
            Action::Pin(package) => self.submit(Request::PinAdd { package }),
            Action::Unpin(id) => self.submit(Request::PinRemove { id }),
            Action::ShowDetails(package) => {
                let request = Request::Show {
                    id: package.id.clone(),
                    source: package.source.clone(),
                };
                self.details = Some(DetailsView {
                    package,
                    result: None,
                });
                self.submit(request);
            }
        }
    }

    fn process_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.worker.events.try_recv() {
            match event {
                WorkerEvent::Queued { id, request } => {
                    if let Some(package_id) = request.package_id() {
                        self.status
                            .insert(package_id.to_string(), PackageStatus::Queued);
                    }
                    self.queue.push(QueuedJob {
                        id,
                        request,
                        running: false,
                    });
                }
                WorkerEvent::Started { id } => {
                    self.progress.clear();
                    self.last_line.clear();
                    self.job_started = Some(Instant::now());
                    self.last_output = self.job_started;
                    if let Some(job) = self.queue.iter_mut().find(|job| job.id == id) {
                        job.running = true;
                        if let Some(package_id) = job.request.package_id() {
                            self.status
                                .insert(package_id.to_string(), PackageStatus::Running);
                        }
                    }
                }
                WorkerEvent::Progress(text) => {
                    self.progress = text;
                    self.last_output = Some(Instant::now());
                }
                WorkerEvent::Log(line) => {
                    self.last_output = Some(Instant::now());
                    // Bei Installationen o. Ä. zeigt die Statusleiste die letzte
                    // Meldung von winget; Abfragen liefern nur Tabellenzeilen.
                    let installing = self
                        .queue
                        .iter()
                        .any(|job| job.running && job.request.changes_packages());
                    let text = line.trim();
                    if installing
                        && !text.is_empty()
                        && !text.starts_with("> ")
                        && !text.contains(['█', '▒'])
                    {
                        self.last_line = text.to_string();
                        // Nach dem Download geht es mit der nächsten Phase weiter.
                        self.progress.clear();
                    }
                    if self.log.len() >= MAX_LOG_LINES {
                        self.log.pop_front();
                    }
                    self.log.push_back(line);
                }
                WorkerEvent::Finished {
                    id,
                    request,
                    result,
                } => {
                    self.queue.retain(|job| job.id != id);
                    self.progress.clear();
                    self.last_line.clear();
                    self.job_started = None;
                    self.last_output = None;
                    self.handle_result(ctx, request, result);
                }
            }
        }

        // Nach Installationen o. Ä. die Listen einmal neu laden, sobald die
        // Warteschlange leer ist.
        if self.needs_refresh && self.queue.is_empty() && !self.leave_elevated {
            self.needs_refresh = false;
            self.submit(self.check_request());
            if self.installed.is_some() {
                self.submit(Request::ListInstalled);
            }
            if self.pins.is_some() {
                self.submit(Request::PinList);
            }
        }
    }

    fn handle_result(
        &mut self,
        ctx: &egui::Context,
        request: Request,
        result: Result<Response, Failure>,
    ) {
        if let Some(package_id) = request.package_id() {
            let status = match &result {
                Ok(Response::Done(Some(note))) => Some(PackageStatus::Done(Some(note.clone()))),
                Ok(_) => Some(PackageStatus::Done(None)),
                // Abgebrochen ist kein Fehler: Zeile wieder normal anzeigen.
                Err(failure) if failure.cancelled => None,
                Err(failure) => Some(PackageStatus::Failed(failure.clone())),
            };
            match status {
                Some(status) => self.status.insert(package_id.to_string(), status),
                None => self.status.remove(package_id),
            };
        }
        if request.changes_packages() && result.is_ok() {
            self.needs_refresh = true;
        }
        // Schlägt ein anderes Update fehl, während das von WPM wartet, soll man
        // die Meldung sehen, bevor sich WPM für sein eigenes Update beendet.
        if self.self_update.is_some()
            && request.changes_packages()
            && matches!(&result, Err(failure) if !failure.cancelled)
        {
            self.self_update_blocked = true;
        }
        if self.hidden
            && let Some(text) = background_result_text(&request, &result)
        {
            self.background_results.push(text);
        }
        let note = match &result {
            Ok(Response::Done(Some(note))) => format!(" {note}"),
            _ => String::new(),
        };
        // Aus dem Infobereich gestartete Suche bei verstecktem Fenster.
        let announce = matches!(request, Request::CheckUpdates { .. })
            && std::mem::take(&mut self.announce_check)
            && self.hidden;
        if announce
            && let Err(failure) = &result
            && !failure.cancelled
        {
            system::notify(
                tr!("Update-Suche fehlgeschlagen", "Update check failed"),
                &failure.summary,
            );
        }

        match (request, result) {
            (Request::Show { id, .. }, result) => {
                if let Some(view) = self.details.as_mut().filter(|v| v.package.id == id) {
                    view.result = Some(result.and_then(|r| match r {
                        Response::Details(details) => Ok(details),
                        _ => Err(Failure::new(tr!(
                            "Unerwartete Antwort",
                            "Unexpected response"
                        ))),
                    }));
                }
            }
            (Request::Version, Ok(Response::Version(version))) => {
                self.winget_version = Some(Ok(version));
            }
            (Request::Version, Err(failure)) => {
                self.winget_version = Some(Err(failure.summary.clone()));
                self.notice_failure(&Request::Version, &failure);
            }
            (Request::ListInstalled, Ok(Response::Installed(mut packages))) => {
                packages.sort_by_cached_key(|p| p.name.to_lowercase());
                self.installed = Some(packages);
            }
            (Request::CheckUpdates { .. }, Ok(Response::Updates(list))) => {
                if announce {
                    announce_updates(&list);
                }
                self.report_pending_upgrades(&list);
                self.resume_after_restart(&list);
                self.selected
                    .retain(|id| list.upgrades.iter().any(|p| &p.id == id));
                self.status
                    .retain(|_, s| !matches!(s, PackageStatus::Done(_)));
                let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
                if !list.upgrades.is_empty() && !focused && !self.hidden {
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                        egui::UserAttentionType::Informational,
                    ));
                }
                self.updates = Some(list);
                self.updates_checked_at = Some(Instant::now());
            }
            (Request::Search { .. }, Ok(Response::SearchResults(results))) => {
                self.search_results = Some(results);
            }
            (Request::Install { package }, Ok(_)) => {
                self.notice(
                    NoticeKind::Success,
                    trf!(
                        "{} wurde installiert.{note}",
                        "{} was installed.{note}",
                        package.name
                    ),
                );
            }
            (Request::Upgrade { package }, Ok(_)) => {
                self.recently_upgraded
                    .insert(package.id.clone(), package.available.clone());
                if let Some(list) = &mut self.updates {
                    list.upgrades.retain(|p| p.id != package.id);
                    list.pinned.retain(|p| p.id != package.id);
                }
                self.selected.remove(&package.id);
                self.notice(
                    NoticeKind::Success,
                    trf!(
                        "{} wurde aktualisiert.{note}",
                        "{} was updated.{note}",
                        package.name
                    ),
                );
            }
            (Request::Uninstall { package }, Ok(_)) => {
                if let Some(installed) = &mut self.installed {
                    installed.retain(|p| p.id != package.id);
                }
                self.notice(
                    NoticeKind::Success,
                    trf!(
                        "{} wurde deinstalliert.{note}",
                        "{} was uninstalled.{note}",
                        package.name
                    ),
                );
            }
            (Request::PinAdd { package }, Ok(_)) => {
                if let Some(list) = &mut self.updates {
                    list.upgrades.retain(|p| p.id != package.id);
                    list.pinned.push(package.clone());
                }
                self.selected.remove(&package.id);
                self.notice(
                    NoticeKind::Info,
                    trf!(
                        "{} ist angeheftet und wird bei „Alle aktualisieren“ übersprungen.",
                        "{} is pinned and will be skipped by “Update all”.",
                        package.name
                    ),
                );
            }
            (Request::PinRemove { id }, Ok(_)) => {
                if let Some(pins) = &mut self.pins {
                    pins.retain(|p| p.id != id);
                }
                self.notice(
                    NoticeKind::Info,
                    trf!("Pin für {id} entfernt.", "Pin removed for {id}."),
                );
            }
            (Request::PinList, Ok(Response::Pins(pins))) => self.pins = Some(pins),
            (Request::SourceList, Ok(Response::Sources(sources))) => self.sources = Some(sources),
            (Request::SourceUpdate, Ok(_)) => {
                self.notice(
                    NoticeKind::Success,
                    tr!("Quellen wurden aktualisiert.", "Sources were updated."),
                );
            }
            (Request::Export { path }, Ok(_)) => {
                self.notice(
                    NoticeKind::Success,
                    trf!(
                        "Paketliste gespeichert: {}",
                        "Package list saved: {}",
                        path.display()
                    ),
                );
            }
            (Request::Import { .. }, Ok(_)) => {
                self.notice(
                    NoticeKind::Success,
                    tr!("Import abgeschlossen.", "Import complete."),
                );
            }
            (request, Err(failure)) if failure.cancelled => {
                self.notice(
                    NoticeKind::Info,
                    trf!("{}: abgebrochen", "{}: cancelled", request.label()),
                );
            }
            (request, Err(failure)) => self.notice_failure(&request, &failure),
            (_, Ok(_)) => {}
        }
    }

    /// Erscheint ein gerade erfolgreich aktualisiertes Paket mit derselben
    /// Version erneut, ist das Update meist noch nicht aktiv (z. B. MSIX-Pakete,
    /// solange die Anwendung läuft) oder winget erkennt die Version falsch.
    fn report_pending_upgrades(&mut self, list: &UpgradeList) {
        let pending: Vec<&str> = list
            .upgrades
            .iter()
            .chain(&list.pinned)
            .filter(|p| self.recently_upgraded.get(&p.id) == Some(&p.available))
            .map(|p| p.name.as_str())
            .collect();
        self.recently_upgraded.clear();
        if !pending.is_empty() {
            let names = pending.join(", ");
            self.notices.push(Notice {
                details: tr!(
                    "winget meldet die Installation als erfolgreich, aktiviert die neue Version \
                     aber erst, wenn die Anwendung (und ggf. ihr Hintergrunddienst) beendet ist \
                     oder Windows neu gestartet wurde. Bleibt das Update danach weiter stehen, \
                     erkennt winget die Version falsch – dann „Anheften“, besonders bei \
                     Programmen, die sich selbst aktualisieren.",
                    "winget reports the installation as successful, but only activates the new version \
                     once the application (and its background service, if any) has been closed \
                     or Windows has been restarted. If the update is still listed after that, \
                     winget detects the version incorrectly – then use “Pin”, especially for \
                     programs that update themselves."
                )
                .into(),
                ..Notice::new(
                    NoticeKind::Info,
                    trf!(
                        "{names}: Update installiert, aber noch nicht aktiv – Anwendung schließen \
                         oder Windows neu starten.",
                        "{names}: Update installed but not active yet – close the application \
                         or restart Windows."
                    ),
                )
            });
        }
    }

    /// Übernimmt geänderte Einstellungen: speichern, Autostart, Theme.
    fn settings_changed(&mut self, ctx: &egui::Context, before: &Settings) {
        if self.settings.autostart != before.autostart && system::AUTOSTART_SUPPORTED {
            let result = system::set_autostart(self.settings.autostart);
            // Die Oberfläche zeigt, was tatsächlich eingetragen ist.
            self.settings.autostart = system::autostart_enabled();
            if let Err(e) = result {
                self.notice(
                    NoticeKind::Error,
                    trf!(
                        "Autostart konnte nicht geändert werden: {e}",
                        "Could not change autostart: {e}"
                    ),
                );
            }
        }
        if self.settings.start_menu != before.start_menu && system::START_MENU_SUPPORTED {
            // Eine eigene Entscheidung: nicht später automatisch wieder anlegen.
            self.settings.start_menu_offered = true;
            let result = system::set_start_menu_entry(self.settings.start_menu);
            self.settings.start_menu = system::start_menu_entry_exists();
            if let Err(e) = result {
                self.notice(
                    NoticeKind::Error,
                    trf!(
                        "Startmenü-Eintrag konnte nicht geändert werden: {e}",
                        "Could not change the Start menu entry: {e}"
                    ),
                );
            }
        }
        if self.settings.theme != before.theme {
            ctx.set_theme(self.settings.theme.to_egui());
        }
        if self.settings.language != before.language {
            // Fenstertitel und Infobereich ziehen beim nächsten Bild nach.
            i18n::set(self.settings.language.resolve());
        }
        if self.settings.tray_icon != before.tray_icon {
            self.sync_tray();
        }
        *self.shared_settings.write().unwrap() = self.settings.clone();
        if let Err(e) = self.settings.save() {
            self.notice(
                NoticeKind::Error,
                trf!(
                    "Einstellungen konnten nicht gespeichert werden: {e}",
                    "Could not save settings: {e}"
                ),
            );
        }
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let count = self.updates.as_ref().map_or(0, |u| u.upgrades.len());
        let title = match count {
            0 => tr!(
                "WPM – Windows-Paketmanager",
                "WPM – Windows Package Manager"
            )
            .to_string(),
            1 => tr!("WPM – 1 Update verfügbar", "WPM – 1 update available").to_string(),
            n => trf!("WPM – {n} Updates verfügbar", "WPM – {n} updates available"),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    // --- Infobereich und Hintergrundbetrieb ---

    /// Legt das Symbol im Infobereich an bzw. entfernt es, passend zur Einstellung.
    fn sync_tray(&mut self) {
        if !(self.settings.tray_icon && tray::SUPPORTED) {
            self.tray = None;
        } else if self.tray.is_none() {
            match Tray::new(&self.command_sender) {
                Ok(tray) => self.tray = Some(tray),
                Err(e) => self.notice(
                    NoticeKind::Error,
                    trf!(
                        "Symbol im Infobereich konnte nicht angelegt werden: {e}",
                        "Could not create the system tray icon: {e}"
                    ),
                ),
            }
        }
    }

    /// Läuft WPM beim Schließen des Fensters im Infobereich weiter?
    fn keeps_running_in_background(&self) -> bool {
        self.tray.is_some() && self.settings.close_to_tray
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        self.leave_elevated = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        if ctx.input(|i| i.viewport().minimized) == Some(true) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        self.hidden = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    /// Beendet WPM ganz, statt es in den Infobereich zu schicken.
    fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        // Symbol sofort entfernen, nicht erst beim Prozessende.
        self.tray = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        ctx.request_repaint();
    }

    fn handle_commands(&mut self, ctx: &egui::Context) {
        while let Ok(command) = self.commands.try_recv() {
            match command {
                Command::Show if !self.quitting => self.show_window(ctx),
                Command::Show => {}
                Command::CheckUpdates => {
                    // Bei verstecktem Fenster kommt das Ergebnis als Benachrichtigung.
                    self.announce_check = self.hidden;
                    self.submit(self.check_request());
                }
                Command::Quit => self.quit(ctx),
            }
        }
    }

    /// Fenster geschlossen: je nach Einstellung im Infobereich weiterlaufen.
    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if self.quitting || !ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        // Ein verstecktes Fenster kann nur ein anderes Programm schließen (z. B.
        // `taskkill` oder ein Updater) – dann wirklich beenden.
        if !self.keeps_running_in_background() || self.hidden {
            self.quitting = true;
            self.tray = None;
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        self.hide_window(ctx);
        // Mit Adminrechten nicht dauerhaft im Hintergrund bleiben: Sobald nichts
        // mehr läuft, übernimmt eine Instanz ohne Adminrechte.
        self.leave_elevated = self.elevated;
        if !self.settings.background_hint_shown {
            system::notify(
                tr!(
                    "WPM läuft im Hintergrund weiter",
                    "WPM keeps running in the background"
                ),
                tr!(
                    "Ein Klick auf das Symbol im Infobereich (unten rechts) öffnet WPM wieder. \
                     Beenden: Rechtsklick auf das Symbol → „Beenden“.",
                    "Click the icon in the system tray (bottom right) to open WPM again. \
                     To quit, right-click the icon and choose “Quit”."
                ),
            );
            let before = self.settings.clone();
            self.settings.background_hint_shown = true;
            self.settings_changed(ctx, &before);
        }
    }

    /// Mit Adminrechten in den Hintergrund geschickt: sobald nichts mehr läuft,
    /// ohne Adminrechte im Infobereich neu starten und sich selbst beenden.
    fn leave_elevated_when_idle(&mut self, ctx: &egui::Context) {
        // Beendet sich WPM schon (z. B. für das Selbst-Update, dessen
        // Hilfsprogramm WPM ohne Adminrechte neu startet), nichts doppelt starten.
        if self.quitting || !self.leave_elevated || !self.queue.is_empty() {
            return;
        }
        self.leave_elevated = false;
        if let Err(e) = system::launch_unelevated(&crate::background_args(self.demo)) {
            system::notify(
                tr!("WPM wurde beendet", "WPM was closed"),
                &trf!(
                    "Mit Administratorrechten bleibt WPM nicht im Hintergrund, und ein Neustart \
                     ohne Adminrechte war nicht möglich ({e}). WPM einfach wieder starten.",
                    "WPM does not stay in the background with administrator rights, and restarting \
                     without admin rights was not possible ({e}). Simply start WPM again."
                ),
            );
        }
        self.quit(ctx);
    }

    fn update_tray(&mut self) {
        if self.tray.is_none() {
            return;
        }
        let count = self.updates.as_ref().map(|u| u.upgrades.len());
        let status = match self.queue.iter().find(|job| job.running) {
            Some(job) => match self.queue.len() - 1 {
                0 => format!("{} …", job.request.label()),
                waiting => format!("{} … (+{waiting})", job.request.label()),
            },
            None => match count {
                None => tr!("Windows-Paketmanager", "Windows Package Manager").to_string(),
                Some(0) => tr!("Keine Updates verfügbar", "No updates available").to_string(),
                Some(1) => tr!("1 Update verfügbar", "1 update available").to_string(),
                Some(n) => trf!("{n} Updates verfügbar", "{n} updates available"),
            },
        };
        let state = tray::State {
            lang: i18n::current(),
            status,
            badge: count.is_some_and(|n| n > 0),
            checking: self.busy(|r| matches!(r, Request::CheckUpdates { .. })),
        };
        if let Some(tray) = &mut self.tray {
            tray.update(state);
        }
    }

    /// Bei verstecktem Fenster: eine Benachrichtigung, sobald alle
    /// Installationen o. Ä. durch sind – vor allem, wenn etwas fehlschlug.
    fn notify_background_results(&mut self) {
        if !self.hidden {
            self.background_results.clear();
            return;
        }
        if self.background_results.is_empty() || !self.queue.is_empty() {
            return;
        }
        let results = std::mem::take(&mut self.background_results);
        let failed: Vec<&str> = results
            .iter()
            .filter(|(_, ok)| !ok)
            .map(|(text, _)| text.as_str())
            .collect();
        if failed.is_empty() {
            let done: Vec<&str> = results.iter().map(|(text, _)| text.as_str()).collect();
            system::notify(tr!("WPM: Fertig", "WPM: Done"), &summarize(&done));
        } else {
            system::notify(
                &trf!(
                    "WPM: {} von {} fehlgeschlagen",
                    "WPM: {} of {} failed",
                    failed.len(),
                    results.len()
                ),
                &trf!(
                    "{} – Details im WPM-Fenster.",
                    "{} – details in the WPM window.",
                    summarize(&failed)
                ),
            );
        }
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let update_count = self.updates.as_ref().map_or(0, |u| u.upgrades.len());
        let updates_label = if update_count > 0 {
            format!("⬆ Updates ({update_count})")
        } else {
            "⬆ Updates".to_string()
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("📦 WPM").heading().strong());
            ui.add_space(16.0);
            let tabs = [
                (Tab::Updates, updates_label),
                (
                    Tab::Installed,
                    tr!("💻 Installiert", "💻 Installed").to_string(),
                ),
                (Tab::Search, tr!("🔍 Suchen", "🔍 Search").to_string()),
                (
                    Tab::Settings,
                    tr!("⚙ Einstellungen", "⚙ Settings").to_string(),
                ),
                (Tab::Log, tr!("📄 Protokoll", "📄 Log").to_string()),
            ];
            for (tab, label) in tabs {
                if ui
                    .selectable_label(self.tab == tab, RichText::new(label).size(15.0))
                    .clicked()
                {
                    self.focus_search = tab == Tab::Search && self.tab != Tab::Search;
                    self.tab = tab;
                }
            }
            if self.demo {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(tr!("Demo-Modus", "Demo mode"))
                            .color(ui.visuals().warn_fg_color),
                    );
                });
            }
        });
    }

    fn notices_ui(&mut self, ui: &mut egui::Ui) {
        self.notices.retain(|n| {
            n.kind == NoticeKind::Error
                || n.admin_hint
                || n.persistent
                || n.created.elapsed() < Duration::from_secs(8)
        });
        // Höchstens drei Fehler und vier Meldungen insgesamt. Entfernt werden
        // zuerst ältere Hinweise, nie die neueste Meldung.
        while self
            .notices
            .iter()
            .filter(|n| n.kind == NoticeKind::Error)
            .count()
            > 3
        {
            let oldest_error = self
                .notices
                .iter()
                .position(|n| n.kind == NoticeKind::Error)
                .unwrap_or(0);
            self.notices.remove(oldest_error);
        }
        while self.notices.len() > 4 {
            let newest = self.notices.len() - 1;
            let index = self.notices[..newest]
                .iter()
                .position(|n| n.kind != NoticeKind::Error)
                .unwrap_or(0);
            self.notices.remove(index);
        }
        if self
            .notices
            .iter()
            .any(|n| n.kind != NoticeKind::Error && !n.admin_hint && !n.persistent)
        {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }

        let can_restart = self.can_restart_elevated();
        let mut dismissed = None;
        let mut restart: Option<Vec<String>> = None;
        let mut pin: Option<Package> = None;
        let mut open_log: Option<std::path::PathBuf> = None;
        for (index, notice) in self.notices.iter_mut().enumerate() {
            let (icon, color) = match notice.kind {
                NoticeKind::Info => ("ℹ", ui.visuals().hyperlink_color),
                NoticeKind::Success => ("✔", widgets::GREEN),
                NoticeKind::Error => ("✖", ui.visuals().error_fg_color),
            };
            let show_admin = notice.admin_hint && system::ELEVATION_SUPPORTED && !self.elevated;
            egui::Frame::new()
                .fill(color.gamma_multiply(0.12))
                .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.5)))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon).color(color).strong());
                        // Erst die Knöpfe rechts, dann der Text im verbleibenden Platz,
                        // damit lange Meldungen umbrechen statt die Knöpfe zu überdecken.
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            if ui
                                .small_button("✖")
                                .on_hover_text(tr!("Schließen", "Close"))
                                .clicked()
                            {
                                dismissed = Some(index);
                            }
                            if let Some(package) = &notice.pin
                                && ui
                                    .small_button(tr!("📌 Anheften", "📌 Pin"))
                                    .on_hover_text(tr!(
                                        "Dieses Update künftig überspringen (winget pin). \
                                         Lässt sich unter „Angeheftete Pakete“ wieder lösen.",
                                        "Skip this update from now on (winget pin). \
                                         You can unpin it again under “Pinned packages”."
                                    ))
                                    .clicked()
                            {
                                pin = Some(package.clone());
                                dismissed = Some(index);
                            }
                            if let Some(path) = notice.log_path.as_ref().filter(|p| p.is_file())
                                && ui
                                    .small_button(tr!("📄 Protokoll öffnen", "📄 Open log"))
                                    .on_hover_text(path.display().to_string())
                                    .clicked()
                            {
                                open_log = Some(path.clone());
                            }
                            if !notice.details.is_empty() {
                                let label = if notice.expanded {
                                    tr!("Details ausblenden", "Hide details")
                                } else {
                                    "Details"
                                };
                                if ui.small_button(label).clicked() {
                                    notice.expanded = !notice.expanded;
                                }
                            }
                            let admin_label = if notice.retry.is_some() {
                                tr!(
                                    "🛡 Mit Adminrechten erneut versuchen",
                                    "🛡 Retry with admin rights"
                                )
                            } else {
                                tr!(
                                    "🛡 Als Administrator neu starten",
                                    "🛡 Restart as administrator"
                                )
                            };
                            if show_admin
                                && ui
                                    .add_enabled(
                                        can_restart,
                                        egui::Button::new(admin_label).small(),
                                    )
                                    .on_hover_text(tr!(
                                        "WPM startet nach einer Windows-Bestätigung mit \
                                         Administratorrechten neu – danach laufen alle Installer \
                                         ohne weitere Nachfrage.",
                                        "WPM restarts with administrator rights after a UAC prompt \
                                         – from then on, all installers run without further prompts."
                                    ))
                                    .on_disabled_hover_text(tr!(
                                        "Erst möglich, wenn keine Vorgänge mehr laufen \
                                         (oder nach „Abbrechen“).",
                                        "Only possible once no operations are running \
                                         (or after “Cancel”)."
                                    ))
                                    .clicked()
                            {
                                restart = Some(notice.retry.iter().map(|p| p.id.clone()).collect());
                            }
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::TOP).with_main_wrap(true),
                                |ui| {
                                    ui.add(egui::Label::new(&notice.title).wrap());
                                },
                            );
                        });
                    });
                    if !notice.advice.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&notice.advice).weak()).wrap());
                    }
                    if notice.expanded {
                        ui.add(
                            egui::Label::new(RichText::new(&notice.details).monospace().size(11.5))
                                .wrap()
                                .selectable(true),
                        );
                    }
                });
        }
        if let Some(index) = dismissed {
            self.notices.remove(index);
        }
        if let Some(upgrade_ids) = restart {
            self.restart_elevated(ui.ctx(), &upgrade_ids);
        }
        if let Some(package) = pin {
            self.apply(Action::Pin(package));
        }
        if let Some(path) = open_log
            && let Err(e) = system::open_log_file(&path)
        {
            self.notice(
                NoticeKind::Error,
                trf!(
                    "Protokoll konnte nicht geöffnet werden: {e}",
                    "Could not open the log: {e}"
                ),
            );
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let running = self.queue.iter().find(|job| job.running);
            if let Some(job) = running {
                ui.spinner();
                ui.label(job.request.label());
                let waiting = self.queue.len() - 1;
                if waiting > 0 {
                    ui.label(
                        RichText::new(trf!(
                            "(+{waiting} in Warteschlange)",
                            "(+{waiting} in queue)"
                        ))
                        .weak(),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(tr!("⏹ Abbrechen", "⏹ Cancel"))
                        .on_hover_text(tr!(
                            "Laufenden Vorgang abbrechen (auch einen hängenden Installer) \
                             und Warteschlange leeren",
                            "Cancel the running operation (including a stuck installer) \
                             and clear the queue"
                        ))
                        .clicked()
                    {
                        self.worker.cancel_all();
                        // Vorgemerkte Updates gehören zur Warteschlange.
                        for package in self
                            .self_update
                            .take()
                            .into_iter()
                            .chain(std::mem::take(&mut self.elevate_upgrades))
                        {
                            self.status.remove(&package.id);
                        }
                    }
                    if let Some(started) = self.job_started {
                        ui.label(RichText::new(widgets::elapsed(started)).monospace().weak());
                    }
                    // winget meldet ohne Konsole keinen Fortschritt; dann läuft der
                    // Balken unbestimmt und daneben steht die letzte Statuszeile.
                    let fraction = parse::progress_fraction(&self.progress);
                    widgets::progress_bar(ui, fraction, 140.0);
                    let silence = self
                        .last_output
                        .map(|t| t.elapsed())
                        .filter(|d| *d >= SILENCE_HINT && job.request.changes_packages());
                    if let Some(silence) = silence {
                        // Häufigster Grund für „hängt“: Der Installer wartet in einem
                        // eigenen Fenster auf eine Eingabe.
                        ui.add(
                            egui::Label::new(
                                RichText::new(trf!(
                                    "⚠ Seit {} Min. keine Rückmeldung – wartet der Installer?",
                                    "⚠ No response for {} min – is the installer waiting?",
                                    silence.as_secs() / 60
                                ))
                                .color(ui.visuals().warn_fg_color),
                            )
                            .truncate(),
                        )
                        .on_hover_text(tr!(
                            "Große Installer brauchen manchmal lange. Manche öffnen aber trotz \
                             „Still“ ein eigenes (Konsolen-)Fenster und warten dort auf eine \
                             Eingabe – z. B. rustup-init, wenn Rust-Programme wie cargo oder \
                             rust-analyzer noch laufen. Fenster in der Taskleiste prüfen, das \
                             betroffene Programm schließen oder „Abbrechen“ (beendet auch den \
                             Installer).",
                            "Large installers sometimes take a long time. But some open their own \
                             (console) window despite “Silent” mode and wait there for input – \
                             e.g. rustup-init when Rust programs such as cargo or rust-analyzer \
                             are still running. Check the windows in the taskbar, close the \
                             affected program or click “Cancel” (this also ends the installer)."
                        ));
                    } else {
                        let detail = if fraction.is_some() {
                            self.progress.replace(['█', '▒'], "").trim().to_string()
                        } else {
                            self.last_line.clone()
                        };
                        ui.add(egui::Label::new(RichText::new(detail).weak()).truncate());
                    }
                });
            } else {
                ui.label(tr!("Bereit", "Ready"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.elevated {
                        ui.label(RichText::new("🛡 Administrator").weak())
                            .on_hover_text(tr!(
                                "WPM läuft mit Administratorrechten.",
                                "WPM is running with administrator rights."
                            ));
                    } else if self.can_restart_elevated()
                        && ui
                            .small_button(tr!(
                                "🛡 Als Administrator neu starten",
                                "🛡 Restart as administrator"
                            ))
                            .on_hover_text(tr!(
                                "WPM startet nach einer Windows-Bestätigung mit \
                                 Administratorrechten neu – danach laufen alle Installer \
                                 ohne weitere Nachfrage.",
                                "WPM restarts with administrator rights after a UAC prompt \
                                 – from then on, all installers run without further prompts."
                            ))
                            .clicked()
                    {
                        let ctx = ui.ctx().clone();
                        self.restart_elevated(&ctx, &[]);
                    }
                    match &self.winget_version {
                        Some(Ok(version)) => {
                            ui.label(RichText::new(format!("winget {version}")).weak())
                        }
                        Some(Err(_)) => ui.label(
                            RichText::new(tr!("winget nicht verfügbar", "winget not available"))
                                .color(ui.visuals().error_fg_color),
                        ),
                        None => ui.label(""),
                    };
                });
            }
        });
    }
}

impl eframe::App for WpmApp {
    // Läuft auch bei verstecktem Fenster, sobald etwas passiert (Ereignisse
    // vom Worker, Klick aufs Symbol im Infobereich, erneuter Programmstart).
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.first_frame {
            self.first_frame = false;
            if self.start_minimized {
                // Mit Symbol im Infobereich ganz unsichtbar, sonst in der Taskleiste.
                if self.tray.is_some() {
                    self.hide_window(ctx);
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
            }
        }
        self.process_events(ctx);
        self.handle_commands(ctx);
        self.handle_close_request(ctx);
        // Selbst-Update vor dem Neustart mit Adminrechten prüfen: Wird dessen
        // Bestätigung abgelehnt, reicht start_elevated_upgrades die Updates per
        // submit() weiter – in self.queue stehen sie erst nach dem nächsten
        // process_events, und so lange muss das Selbst-Update warten.
        self.start_self_update(ctx);
        // Gesammelte Updates: Neustart mit Adminrechten, sobald nichts mehr installiert.
        self.start_elevated_upgrades(ctx);
        self.leave_elevated_when_idle(ctx);
        self.update_tray();
        self.notify_background_results();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Versteckt nichts zeichnen – das spart auch die ständigen Neuzeichnungen
        // durch Ladeanimationen.
        if self.hidden {
            return;
        }
        let ctx = ui.ctx().clone();
        self.update_window_title(&ctx);

        egui::Panel::top("tabs").show(ui, |ui| {
            ui.add_space(6.0);
            self.tab_bar(ui);
            ui.add_space(4.0);
            self.notices_ui(ui);
            if !self.notices.is_empty() {
                ui.add_space(4.0);
            }
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.add_space(3.0);
            self.status_bar(ui);
            ui.add_space(1.0);
        });

        let settings_before = self.settings.clone();
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Updates => self.updates_tab(ui),
            Tab::Installed => self.installed_tab(ui),
            Tab::Search => self.search_tab(ui),
            Tab::Settings => self.settings_tab(ui),
            Tab::Log => self.log_tab(ui),
        });
        if self.settings != settings_before {
            self.settings_changed(&ctx, &settings_before);
        }

        self.details_window(&ctx);
        self.confirm_uninstall_dialog(&ctx);
        self.licenses_window(&ctx);
    }
}

/// Text für die Benachrichtigung nach Vorgängen bei verstecktem Fenster;
/// `None` für Abfragen und abgebrochene Vorgänge.
fn background_result_text(
    request: &Request,
    result: &Result<Response, Failure>,
) -> Option<(String, bool)> {
    let ok = match result {
        Ok(_) => true,
        Err(failure) if failure.cancelled => return None,
        Err(_) => false,
    };
    let text = match request {
        Request::Install { .. }
        | Request::Upgrade { .. }
        | Request::Uninstall { .. }
        | Request::Import { .. }
            if !ok =>
        {
            request.label()
        }
        Request::Install { package } => {
            trf!("{} installiert", "{} installed", package.name)
        }
        Request::Upgrade { package } => trf!("{} aktualisiert", "{} updated", package.name),
        Request::Uninstall { package } => {
            trf!("{} deinstalliert", "{} uninstalled", package.name)
        }
        Request::Import { .. } => tr!("Import abgeschlossen", "Import complete").to_string(),
        _ => return None,
    };
    Some((text, ok))
}

/// Benachrichtigung mit dem Ergebnis einer Update-Suche – auch wenn es
/// nichts Neues gibt (Suche wurde ausdrücklich gestartet).
fn announce_updates(list: &UpgradeList) {
    let names: Vec<&str> = list.upgrades.iter().map(|p| p.name.as_str()).collect();
    match names.len() {
        0 => system::notify(
            tr!("Keine Updates verfügbar", "No updates available"),
            tr!("Alle Pakete sind aktuell.", "All packages are up to date."),
        ),
        1 => system::notify(
            tr!("1 Update verfügbar", "1 update available"),
            &summarize(&names),
        ),
        n => system::notify(
            &trf!("{n} Updates verfügbar", "{n} updates available"),
            &summarize(&names),
        ),
    }
}

/// Höchstens vier Einträge, danach „…“.
fn summarize(items: &[&str]) -> String {
    let mut text = items.iter().take(4).copied().collect::<Vec<_>>().join(", ");
    if items.len() > 4 {
        text.push_str(", …");
    }
    text
}

fn setup_style(ctx: &egui::Context) {
    use egui::{FontFamily, FontId, TextStyle};
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(20.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
            (
                TextStyle::Small,
                FontId::new(11.0, FontFamily::Proportional),
            ),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;

    #[test]
    fn background_result_text_in_both_languages() {
        let request = Request::Upgrade {
            package: Package {
                name: "Firefox".into(),
                ..Package::default()
            },
        };
        let ok: Result<Response, Failure> = Ok(Response::Done(None));
        assert_eq!(
            background_result_text(&request, &ok),
            Some(("Firefox aktualisiert".to_string(), true))
        );
        i18n::with(Lang::En, || {
            assert_eq!(
                background_result_text(&request, &ok),
                Some(("Firefox updated".to_string(), true))
            );
        });
    }
}
