//! Hintergrund-Thread, der winget-Aufträge nacheinander abarbeitet.
//!
//! winget verträgt keine parallelen Installationen, deshalb gibt es genau
//! eine Warteschlange. Ergebnisse gehen als [`WorkerEvent`] an die Oberfläche.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;

use crate::runner::{OutputEvent, Runner};
use crate::system;
use crate::winget::{Failure, Options, Request, Response, UpgradeList};

pub type JobId = u64;

pub struct Job {
    pub id: JobId,
    pub request: Request,
    pub options: Options,
    /// Bei gefundenen Updates eine Windows-Benachrichtigung zeigen.
    pub notify: bool,
    generation: u64,
}

pub enum WorkerEvent {
    Queued {
        id: JobId,
        request: Request,
    },
    Started {
        id: JobId,
    },
    Progress(String),
    Log(String),
    Finished {
        id: JobId,
        request: Request,
        result: Result<Response, Failure>,
    },
}

/// Zum Einreihen von Aufträgen; kann geklont und an andere Threads gegeben werden.
#[derive(Clone)]
pub struct JobSender {
    jobs: Sender<Job>,
    events: Sender<WorkerEvent>,
    next_id: Arc<AtomicU64>,
    generation: Arc<AtomicU64>,
    ctx: egui::Context,
}

impl JobSender {
    pub fn submit(&self, request: Request, options: Options, notify: bool) -> JobId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let _ = self.events.send(WorkerEvent::Queued {
            id,
            request: request.clone(),
        });
        let _ = self.jobs.send(Job {
            id,
            request,
            options,
            notify,
            generation: self.generation.load(Ordering::SeqCst),
        });
        self.ctx.request_repaint();
        id
    }
}

pub struct Worker {
    pub sender: JobSender,
    pub events: Receiver<WorkerEvent>,
    runner: Arc<dyn Runner>,
    /// Unix-Zeit (s) der letzten erfolgreichen Update-Prüfung, 0 = noch nie.
    pub last_check: Arc<AtomicU64>,
}

impl Worker {
    pub fn spawn(runner: Arc<dyn Runner>, ctx: egui::Context) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (event_tx, event_rx) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let last_check = Arc::new(AtomicU64::new(0));

        let sender = JobSender {
            jobs: job_tx,
            events: event_tx.clone(),
            next_id: Arc::new(AtomicU64::new(1)),
            generation: generation.clone(),
            ctx: ctx.clone(),
        };

        let thread_runner = runner.clone();
        let thread_last_check = last_check.clone();
        thread::Builder::new()
            .name("winget-worker".into())
            .spawn(move || {
                let mut notified = HashSet::new();
                for job in job_rx {
                    let send = |event: WorkerEvent| {
                        let _ = event_tx.send(event);
                        ctx.request_repaint();
                    };
                    if job.generation < generation.load(Ordering::SeqCst) {
                        send(WorkerEvent::Finished {
                            id: job.id,
                            request: job.request,
                            result: Err(Failure::cancelled()),
                        });
                        continue;
                    }
                    send(WorkerEvent::Started { id: job.id });
                    let mut result = execute(thread_runner.as_ref(), &job, &send);
                    if job.generation < generation.load(Ordering::SeqCst) {
                        result = Err(Failure::cancelled());
                    }
                    if let (Request::CheckUpdates { .. }, Ok(Response::Updates(list))) =
                        (&job.request, &result)
                    {
                        thread_last_check.store(unix_now(), Ordering::Relaxed);
                        notify_new_updates(list, &mut notified, job.notify);
                    }
                    send(WorkerEvent::Finished {
                        id: job.id,
                        request: job.request,
                        result,
                    });
                }
            })
            .expect("Worker-Thread konnte nicht gestartet werden");

        Self {
            sender,
            events: event_rx,
            runner,
            last_check,
        }
    }

    /// Bricht den laufenden Auftrag ab und verwirft die Warteschlange.
    pub fn cancel_all(&self) {
        self.sender.generation.fetch_add(1, Ordering::SeqCst);
        self.runner.cancel();
    }
}

fn execute(
    runner: &dyn Runner,
    job: &Job,
    send: &dyn Fn(WorkerEvent),
) -> Result<Response, Failure> {
    let args = job.request.args(&job.options);
    send(WorkerEvent::Log(format!("> winget {}", quote_args(&args))));
    let output = runner
        .run(&args, &mut |event| match event {
            OutputEvent::Line(line) => send(WorkerEvent::Log(line)),
            OutputEvent::Progress(text) => send(WorkerEvent::Progress(text)),
        })
        .map_err(|e| {
            Failure::new(if e.kind() == std::io::ErrorKind::NotFound {
                "winget wurde nicht gefunden. Bitte den „App-Installer“ aus dem Microsoft Store \
                 installieren oder aktualisieren."
                    .to_string()
            } else {
                format!("winget konnte nicht gestartet werden: {e}")
            })
        })?;
    if output.exit_code.is_none() {
        return Err(Failure::cancelled());
    }
    job.request.interpret(&output)
}

/// Benachrichtigt nur über Updates, die seit der letzten Prüfung neu sind.
fn notify_new_updates(list: &UpgradeList, notified: &mut HashSet<(String, String)>, notify: bool) {
    let current: HashSet<(String, String)> = list
        .upgrades
        .iter()
        .map(|p| (p.id.clone(), p.available.clone()))
        .collect();
    let new: Vec<_> = list
        .upgrades
        .iter()
        .filter(|p| !notified.contains(&(p.id.clone(), p.available.clone())))
        .collect();
    if notify && !new.is_empty() {
        let title = match list.upgrades.len() {
            1 => "1 Update verfügbar".to_string(),
            n => format!("{n} Updates verfügbar"),
        };
        let mut names: Vec<&str> = new.iter().take(4).map(|p| p.name.as_str()).collect();
        if new.len() > names.len() {
            names.push("…");
        }
        system::notify(&title, &names.join(", "));
    }
    *notified = current;
}

fn quote_args(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if a.contains(' ') {
                format!("\"{a}\"")
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
