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
    /// Administrator-Modus ist eingestellt, die Windows-Bestätigung wurde aber abgelehnt.
    pub elevation_declined: bool,
    /// Läuft mit Adminrechten eines anderen Benutzerkontos.
    pub other_account: bool,
}

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
    progress: String,

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
    other_account: bool,
    /// Erfolgreich aktualisierte Pakete (ID → Version) bis zur nächsten Prüfung.
    recently_upgraded: HashMap<String, String>,
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
        // Eintrag wird mit dem aktuellen Pfad erneuert (falls die EXE verschoben
        // wurde) und auf die passende Variante umgestellt: Aufgabenplanung im
        // Administrator-Modus, sonst der Run-Schlüssel. Ohne Adminrechte bleibt
        // eine vorhandene Aufgabe unangetastet (sie lässt sich dann nicht ändern).
        if system::AUTOSTART_SUPPORTED {
            settings.autostart = system::autostart_enabled();
            let use_task = elevated_task_possible(&settings, flags.elevated, flags.other_account);
            let may_change = !flags.other_account && (flags.elevated || !settings.run_as_admin);
            if settings.autostart && may_change {
                let _ = system::configure_autostart(true, use_task);
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
            other_account: flags.other_account,
            recently_upgraded: HashMap::new(),
            window: system::native_window(cc),
            start_minimized: flags.minimized,
            first_frame: true,
            demo: flags.demo,
            window_title: String::new(),
        };

        if flags.elevation_declined {
            app.notices.push(Notice {
                admin_hint: true,
                ..Notice::new(
                    NoticeKind::Info,
                    "Die Windows-Bestätigung wurde abgelehnt – WPM läuft diesmal ohne Administratorrechte.",
                )
            });
        }
        if flags.other_account {
            app.notice(
                NoticeKind::Error,
                "WPM läuft mit den Adminrechten eines anderen Benutzerkontos. Programme, \
                 Einstellungen und Autostart beziehen sich jetzt auf dieses Konto – dafür ist der \
                 Administrator-Modus nicht gedacht.",
            );
        }
        app.submit(Request::Version);
        if app.settings.check_on_startup {
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

    fn notice_failure(&mut self, label: &str, failure: &Failure) {
        self.notices.push(Notice {
            details: failure.details.clone(),
            admin_hint: failure.needs_admin,
            ..Notice::new(
                NoticeKind::Error,
                format!("{label} fehlgeschlagen: {}", failure.summary),
            )
        });
    }

    /// Startet WPM mit Administratorrechten neu (Windows fragt einmal nach).
    fn restart_elevated(&mut self, ctx: &egui::Context) {
        let mut args = crate::relaunch_flags();
        if self.demo {
            args.push("--demo".into());
        }
        match system::relaunch_elevated(&args, self.window) {
            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(e) => self.notice(
                NoticeKind::Error,
                format!("Neustart mit Administratorrechten nicht möglich: {e}"),
            ),
        }
    }

    /// Ein Neustart verwirft laufende Vorgänge, deshalb nur im Leerlauf.
    fn can_restart_elevated(&self) -> bool {
        system::ELEVATION_SUPPORTED && !self.elevated && self.queue.is_empty()
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
                self.notice_failure("winget-Version abfragen", &failure);
            }
            (Request::ListInstalled, Ok(Response::Installed(mut packages))) => {
                packages.sort_by_cached_key(|p| p.name.to_lowercase());
                self.installed = Some(packages);
            }
            (Request::CheckUpdates { .. }, Ok(Response::Updates(list))) => {
                self.report_pending_upgrades(&list);
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
            (request, Err(failure)) => self.notice_failure(&request.label(), &failure),
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
        let autostart_changed = self.settings.autostart != before.autostart;
        let admin_changed = self.settings.run_as_admin != before.run_as_admin;
        if system::AUTOSTART_SUPPORTED
            && (autostart_changed || (admin_changed && self.settings.autostart))
        {
            // Die Aufgabenplanung (Start ohne Nachfrage) lässt sich nur mit
            // Adminrechten einrichten; bis dahin dient der Run-Schlüssel.
            let use_task =
                elevated_task_possible(&self.settings, self.elevated, self.other_account);
            let result = system::configure_autostart(self.settings.autostart, use_task);
            // Die Oberfläche zeigt, was tatsächlich eingetragen ist.
            self.settings.autostart = system::autostart_enabled();
            if let Err(e) = result {
                self.notice(
                    NoticeKind::Error,
                    format!("Autostart konnte nicht geändert werden: {e}"),
                );
            } else if self.settings.autostart
                && self.settings.run_as_admin
                && !self.elevated
                && system::elevated_autostart_allowed()
            {
                self.notices.push(Notice {
                    admin_hint: true,
                    ..Notice::new(
                        NoticeKind::Info,
                        "Autostart ist eingerichtet. Damit beim Anmelden keine Windows-Bestätigung \
                         erscheint, WPM einmal als Administrator neu starten.",
                    )
                });
            }
        }
        if admin_changed && self.settings.run_as_admin && !self.elevated && !self.settings.autostart
        {
            self.notices.push(Notice {
                admin_hint: true,
                ..Notice::new(
                    NoticeKind::Info,
                    "Der Administrator-Modus gilt ab dem nächsten Start von WPM.",
                )
            });
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
        if self.notices.len() > 4 {
            self.notices.drain(..self.notices.len() - 4);
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
        let mut restart = false;
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
                            if show_admin
                                && ui
                                    .add_enabled(
                                        can_restart,
                                        egui::Button::new("🛡 Als Administrator neu starten")
                                            .small(),
                                    )
                                    .on_hover_text(
                                        "Windows fragt dann einmal nach – danach laufen alle \
                                         Installer ohne weitere Bestätigung.",
                                    )
                                    .on_disabled_hover_text(
                                        "Erst möglich, wenn keine Vorgänge mehr laufen \
                                         (oder nach „Abbrechen“).",
                                    )
                                    .clicked()
                            {
                                restart = true;
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
        if restart {
            self.restart_elevated(ui.ctx());
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
                    ui.add(
                        egui::Label::new(RichText::new(&self.progress).monospace().weak())
                            .truncate(),
                    );
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
                                "Windows fragt einmal nach – danach laufen alle Installer \
                                 ohne weitere Bestätigung.",
                            )
                            .clicked()
                    {
                        let ctx = ui.ctx().clone();
                        self.restart_elevated(&ctx);
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

/// Autostart über die Aufgabenplanung (ohne Nachfrage beim Anmelden) nur im
/// Administrator-Modus, mit Adminrechten des eigenen Kontos und wenn wpm.exe
/// in einem geschützten Ordner liegt.
fn elevated_task_possible(settings: &Settings, elevated: bool, other_account: bool) -> bool {
    settings.run_as_admin && elevated && !other_account && system::elevated_autostart_allowed()
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
