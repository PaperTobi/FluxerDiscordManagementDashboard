//! The live settings: one tree in memory, resolved once per change ([`SettingsView`]), changed only through
//! [`SettingsService::change`] (files written, the new view published, the change audited in the event log).

use std::sync::Arc;

use pb_settings::{Change, SettingError, SettingsTree, SettingsView};
use pb_store_api::{Actor, Event, EventLog, SettingsChanged, SettingsFiles, StoreError};
use tokio::sync::{Mutex, watch};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ChangeError {
    #[error(transparent)]
    Setting(#[from] SettingError),
    #[error("the settings could not be saved: {0}")]
    Store(#[from] StoreError),
}

/// The settings.
pub struct SettingsService {
    view: watch::Sender<Arc<SettingsView>>,
    files: Arc<dyn SettingsFiles>,
    log: Arc<dyn EventLog>,
    write: Mutex<()>,
}

impl std::fmt::Debug for SettingsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsService").finish_non_exhaustive()
    }
}

impl SettingsService {
    pub fn new(tree: SettingsTree, files: Arc<dyn SettingsFiles>, log: Arc<dyn EventLog>) -> SettingsService {
        SettingsService {
            view: watch::Sender::new(Arc::new(SettingsView::new(tree))),
            files,
            log,
            write: Mutex::new(()),
        }
    }

    /// The current settings (cheap; a snapshot that does not change).
    pub fn current(&self) -> Arc<SettingsView> {
        self.view.borrow().clone()
    }

    /// Sees every change.
    pub fn watch(&self) -> watch::Receiver<Arc<SettingsView>> {
        self.view.subscribe()
    }

    fn swap(&self, tree: SettingsTree) {
        self.view.send_replace(Arc::new(SettingsView::new(tree)));
    }

    /// Changes the settings: `f` edits a copy (validating) and returns the changes; they are written to the files,
    /// applied, and audited. Nothing changes when `f` fails or writes fail.
    pub async fn change<F>(&self, by: Actor, f: F) -> Result<Vec<Change>, ChangeError>
    where
        F: FnOnce(&mut SettingsTree) -> Result<Vec<Change>, SettingError>,
    {
        let _guard = self.write.lock().await;
        let mut tree = self.current().tree().clone();
        let changes = f(&mut tree)?;
        if changes.is_empty() {
            return Ok(changes);
        }
        self.files.write(&changes).await?;
        self.swap(tree);
        let events: Vec<_> = changes
            .iter()
            .filter_map(|c| {
                Event::SettingsChanged(Box::new(SettingsChanged {
                    change: c.clone(),
                    by: by.clone(),
                }))
                .to_new(None)
            })
            .collect();
        if let Err(e) = self.log.append(events).await {
            // The files are the truth and already changed; only the audit entry is missing (the log shows why).
            tracing::error!(error = %e, "a settings change could not be recorded in the event log");
        }
        Ok(changes)
    }

    /// Replaces everything after the files were edited by hand (`reload`).
    pub async fn reload(&self) -> Result<Vec<pb_settings::FileError>, ChangeError> {
        let _guard = self.write.lock().await;
        let (mut tree, problems) = self.files.load().await?;
        if problems.is_empty() {
            tree.file_defaults = self.current().file_defaults.clone();
            self.swap(tree);
        }
        Ok(problems)
    }
}
