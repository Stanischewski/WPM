//! Prüft im Hintergrund regelmäßig auf Updates – unabhängig davon, ob das
//! Fenster gerade minimiert ist.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use crate::settings::Settings;
use crate::winget::{Options, Request};
use crate::worker::{JobSender, unix_now};

pub fn spawn(settings: Arc<RwLock<Settings>>, jobs: JobSender, last_check: Arc<AtomicU64>) {
    let started = unix_now();
    thread::Builder::new()
        .name("update-scheduler".into())
        .spawn(move || {
            let mut last_submitted = started;
            loop {
                thread::sleep(Duration::from_secs(60));
                let settings = settings.read().unwrap().clone();
                if !settings.auto_check {
                    continue;
                }
                let interval = u64::from(settings.check_interval_hours.max(1)) * 3600;
                // Auch manuelle Prüfungen verschieben die nächste automatische.
                let last = last_submitted.max(last_check.load(Ordering::Relaxed));
                let now = unix_now();
                if now >= last + interval {
                    last_submitted = now;
                    jobs.submit(
                        Request::CheckUpdates {
                            include_unknown: settings.include_unknown,
                        },
                        Options::default(),
                        settings.notify,
                    );
                }
            }
        })
        .expect("Scheduler-Thread konnte nicht gestartet werden");
}
