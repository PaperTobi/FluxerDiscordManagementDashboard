//! How the engine's own parts are doing (for `/healthz` and the System page).

/// The engine's long-lived parts and whether one of them failed for good.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineHealth {
    pub actors: Vec<ActorHealth>,
    /// A part that could not go on: the bot must be restarted.
    pub fatal: Option<FatalError>,
}

impl EngineHealth {
    /// Something keeps the bot from working (a part failed for good or stopped answering).
    pub fn failing(&self) -> bool {
        self.fatal.is_some()
            || self
                .actors
                .iter()
                .any(|a| matches!(a.state, ActorState::Failed | ActorState::NotAnswering))
    }

    /// A part is being started again after a crash (the bot works, maybe not completely).
    pub fn degraded(&self) -> bool {
        self.actors
            .iter()
            .any(|a| matches!(a.state, ActorState::Restarting { .. }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorHealth {
    pub name: &'static str,
    pub state: ActorState,
    /// Times it was started again after a crash.
    pub restarts: u32,
    /// Messages waiting for it.
    pub queued: u64,
    /// Milliseconds since it last took a message or did its own work.
    pub idle_ms: u64,
    /// Why it last crashed.
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorState {
    Running,
    /// Crashed; starts again after a pause.
    Restarting {
        attempt: u32,
    },
    /// Messages wait and it has not taken one for a minute.
    NotAnswering,
    /// Ended on its own (its inputs closed) or at shutdown.
    Stopped,
    /// Crashed and is not started again.
    Failed,
}

/// A part of the engine that could not go on; the process should end so the service manager starts it again.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{actor} failed: {error}")]
pub struct FatalError {
    pub actor: &'static str,
    pub error: String,
}
