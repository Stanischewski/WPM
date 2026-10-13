//! Zustand und Logik der Oberfläche. Die einzelnen Reiter liegen in eigenen Dateien.

mod details;
mod installed;
mod log;
mod search;
mod settings_tab;
mod updates;
mod widgets;

use std::collections::{HashMap, HashSet, VecDeque};
use std::mem::discriminant;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};

use crate::runner::{Runner, WingetRunner};
use crate::scheduler;
use crate::settings::Settings;
use crate::system;
use crate::winget::demo::DemoRunner;
use crate::winget::parse;
use crate::winget::{
    Details, Failure, Options, Package, Pin, Request, Response, Source, UpgradeList,
};
use crate::worker::{JobId, Worker, WorkerEvent};

const MAX_LOG_LINES: usize = 5000;

pub struct StartupFlags {
    pub demo: bool,
    /// Vom Autostart gestartet bzw. ausdrücklich minimiert gewünscht.
    pub minimized: bool,
    /// WPM läuft mit Administratorrechten.
    pub elevated: bool,
    /// Läuft mit Adminrechten eines anderen Benutzerkontos.
    pub other_account: bool,
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
    Done(String),
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

        let runner: Arc<dyn Runner> = if flags.demo {
            Arc::new(DemoRunner::default())
        } else {
            Arc::new(WingetRunner::default())
        };
        let worker = Worker::spawn(runner, ctx.clone());
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
        };

        if flags.other_account {
            app.notice(
                NoticeKind::Error,
                "WPM läuft nach der Windows-Bestätigung unter einem anderen Benutzerkonto \
                 (andere Zugangsdaten eingegeben oder Windows-Administratorschutz aktiv). \
                 Benutzerbezogene Programme und Einstellungen gehören dann zu diesem Konto.",
            );
        }
        app.submit(Request::Version);
        // Nach einem Neustart mit Adminrechten zuerst die Liste holen, dann
        // die gewünschten Updates fortsetzen (siehe `resume_after_restart`).
        if app.settings.check_on_startup || !app.resume_upgrades.is_empty() {
            let notify = flags.minimized && app.settings.notify;
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
        let retry = match request {
            Request::Upgrade { package } if failure.needs_admin => Some(package.clone()),
            _ => None,
        };
        self.notices.push(Notice {
            details: failure.details.clone(),
            admin_hint: failure.needs_admin,
            retry,
            ..Notice::new(
                NoticeKind::Error,
                format!("{} fehlgeschlagen: {}", request.label(), failure.summary),
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
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                true
            }
            Err(e) => {
                self.notice(
                    NoticeKind::Info,
                    format!("Kein Neustart mit Administratorrechten: {e}"),
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
        let packages = std::mem::take(&mut self.elevate_upgrades);
        let all_available: HashSet<&str> = self
            .updates
            .iter()
            .flat_map(|u| &u.upgrades)
            .map(|p| p.id.as_str())
            .collect();
        let requested: HashSet<&str> = packages.iter().map(|p| p.id.as_str()).collect();
        let ids = if !all_available.is_empty() && requested == all_available {
            vec![ALL_UPGRADES.to_string()]
        } else {
            packages.iter().map(|p| p.id.clone()).collect()
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
                    format!("Kein Update mehr gefunden für: {}", missing.join(", ")),
                );
            }
            found
        };
        for package in packages {
            self.submit(Request::Upgrade { package });
        }
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
                    if let Some(job) = self.queue.iter_mut().find(|job| job.id == id) {
                        job.running = true;
                        if let Some(package_id) = job.request.package_id() {
                            self.status
                                .insert(package_id.to_string(), PackageStatus::Running);
                        }
                    }
                }
                WorkerEvent::Progress(text) => self.progress = text,
                WorkerEvent::Log(line) => {
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
                    self.handle_result(ctx, request, result);
                }
            }
        }

        // Nach Installationen o. Ä. die Listen einmal neu laden, sobald die
        // Warteschlange leer ist.
        if self.needs_refresh && self.queue.is_empty() {
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
                Ok(Response::Done(Some(note))) => Some(PackageStatus::Done(note.clone())),
                Ok(_) => Some(PackageStatus::Done("Erledigt".into())),
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
        let note = match &result {
            Ok(Response::Done(Some(note))) => format!(" {note}"),
            _ => String::new(),
        };

        match (request, result) {
            (Request::Show { id, .. }, result) => {
                if let Some(view) = self.details.as_mut().filter(|v| v.package.id == id) {
                    view.result = Some(result.and_then(|r| match r {
                        Response::Details(details) => Ok(details),
                        _ => Err(Failure::new("Unerwartete Antwort")),
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
                self.report_pending_upgrades(&list);
                self.resume_after_restart(&list);
                self.selected
                    .retain(|id| list.upgrades.iter().any(|p| &p.id == id));
                self.status
                    .retain(|_, s| !matches!(s, PackageStatus::Done(_)));
                let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
                if !list.upgrades.is_empty() && !focused {
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
                    format!("{} wurde installiert.{note}", package.name),
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
                    format!("{} wurde aktualisiert.{note}", package.name),
                );
            }
            (Request::Uninstall { package }, Ok(_)) => {
                if let Some(installed) = &mut self.installed {
                    installed.retain(|p| p.id != package.id);
                }
                self.notice(
                    NoticeKind::Success,
                    format!("{} wurde deinstalliert.{note}", package.name),
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
                    format!(
                        "{} ist angeheftet und wird bei „Alle aktualisieren“ übersprungen.",
                        package.name
                    ),
                );
            }
            (Request::PinRemove { id }, Ok(_)) => {
                if let Some(pins) = &mut self.pins {
                    pins.retain(|p| p.id != id);
                }
                self.notice(NoticeKind::Info, format!("Pin für {id} entfernt."));
            }
            (Request::PinList, Ok(Response::Pins(pins))) => self.pins = Some(pins),
            (Request::SourceList, Ok(Response::Sources(sources))) => self.sources = Some(sources),
            (Request::SourceUpdate, Ok(_)) => {
                self.notice(NoticeKind::Success, "Quellen wurden aktualisiert.");
            }
            (Request::Export { path }, Ok(_)) => {
                self.notice(
                    NoticeKind::Success,
                    format!("Paketliste gespeichert: {}", path.display()),
                );
            }
            (Request::Import { .. }, Ok(_)) => {
                self.notice(NoticeKind::Success, "Import abgeschlossen.");
            }
            (request, Err(failure)) if failure.cancelled => {
                self.notice(
                    NoticeKind::Info,
                    format!("{}: abgebrochen", request.label()),
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
                details: "winget meldet die Installation als erfolgreich, aktiviert die neue Version \
                          aber erst, wenn die Anwendung (und ggf. ihr Hintergrunddienst) beendet ist \
                          oder Windows neu gestartet wurde. Bleibt das Update danach weiter stehen, \
                          erkennt winget die Version falsch – dann „Anheften“, besonders bei \
                          Programmen, die sich selbst aktualisieren."
                    .into(),
                ..Notice::new(
                    NoticeKind::Info,
                    format!(
                        "{names}: Update installiert, aber noch nicht aktiv – Anwendung schließen \
                         oder Windows neu starten."
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
                    format!("Autostart konnte nicht geändert werden: {e}"),
                );
            }
        }
        if self.settings.theme != before.theme {
            ctx.set_theme(self.settings.theme.to_egui());
        }
        *self.shared_settings.write().unwrap() = self.settings.clone();
        if let Err(e) = self.settings.save() {
            self.notice(
                NoticeKind::Error,
                format!("Einstellungen konnten nicht gespeichert werden: {e}"),
            );
        }
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let count = self.updates.as_ref().map_or(0, |u| u.upgrades.len());
        let title = match count {
            0 => "WPM – Windows-Paketmanager".to_string(),
            1 => "WPM – 1 Update verfügbar".to_string(),
            n => format!("WPM – {n} Updates verfügbar"),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
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
                (Tab::Installed, "💻 Installiert".to_string()),
                (Tab::Search, "🔍 Suchen".to_string()),
                (Tab::Settings, "⚙ Einstellungen".to_string()),
                (Tab::Log, "📄 Protokoll".to_string()),
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
                    ui.label(RichText::new("Demo-Modus").color(ui.visuals().warn_fg_color));
                });
            }
        });
    }

    fn notices_ui(&mut self, ui: &mut egui::Ui) {
        self.notices.retain(|n| {
            n.kind == NoticeKind::Error
                || n.admin_hint
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
            .any(|n| n.kind != NoticeKind::Error && !n.admin_hint)
        {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }

        let can_restart = self.can_restart_elevated();
        let mut dismissed = None;
        let mut restart: Option<Vec<String>> = None;
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
                            if ui.small_button("✖").on_hover_text("Schließen").clicked() {
                                dismissed = Some(index);
                            }
                            if !notice.details.is_empty() {
                                let label = if notice.expanded {
                                    "Details ausblenden"
                                } else {
                                    "Details"
                                };
                                if ui.small_button(label).clicked() {
                                    notice.expanded = !notice.expanded;
                                }
                            }
                            let admin_label = if notice.retry.is_some() {
                                "🛡 Mit Adminrechten erneut versuchen"
                            } else {
                                "🛡 Als Administrator neu starten"
                            };
                            if show_admin
                                && ui
                                    .add_enabled(
                                        can_restart,
                                        egui::Button::new(admin_label).small(),
                                    )
                                    .on_hover_text(
                                        "WPM startet nach einer Windows-Bestätigung mit \
                                         Administratorrechten neu – danach laufen alle Installer \
                                         ohne weitere Nachfrage.",
                                    )
                                    .on_disabled_hover_text(
                                        "Erst möglich, wenn keine Vorgänge mehr laufen \
                                         (oder nach „Abbrechen“).",
                                    )
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
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let running = self.queue.iter().find(|job| job.running);
            if let Some(job) = running {
                ui.spinner();
                ui.label(job.request.label());
                let waiting = self.queue.len() - 1;
                if waiting > 0 {
                    ui.label(RichText::new(format!("(+{waiting} in Warteschlange)")).weak());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("⏹ Abbrechen")
                        .on_hover_text("Laufenden Vorgang abbrechen und Warteschlange leeren")
                        .clicked()
                    {
                        self.worker.cancel_all();
                    }
                    if let Some(started) = self.job_started {
                        ui.label(RichText::new(widgets::elapsed(started)).monospace().weak());
                    }
                    // winget meldet ohne Konsole keinen Fortschritt; dann läuft der
                    // Balken unbestimmt und daneben steht die letzte Statuszeile.
                    let fraction = parse::progress_fraction(&self.progress);
                    widgets::progress_bar(ui, fraction, 140.0);
                    let detail = if fraction.is_some() {
                        self.progress.replace(['█', '▒'], "").trim().to_string()
                    } else {
                        self.last_line.clone()
                    };
                    ui.add(egui::Label::new(RichText::new(detail).weak()).truncate());
                });
            } else {
                ui.label("Bereit");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.elevated {
                        ui.label(RichText::new("🛡 Administrator").weak())
                            .on_hover_text("WPM läuft mit Administratorrechten.");
                    } else if self.can_restart_elevated()
                        && ui
                            .small_button("🛡 Als Administrator neu starten")
                            .on_hover_text(
                                "WPM startet nach einer Windows-Bestätigung mit \
                                 Administratorrechten neu – danach laufen alle Installer \
                                 ohne weitere Nachfrage.",
                            )
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
                            RichText::new("winget nicht verfügbar")
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
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_events(ctx);
        // Gesammelte Updates: Neustart mit Adminrechten, sobald nichts mehr installiert.
        self.start_elevated_upgrades(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.first_frame {
            self.first_frame = false;
            if self.start_minimized {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }
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
    }
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
