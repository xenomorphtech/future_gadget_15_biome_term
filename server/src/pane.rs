use crate::event::{now_ms, Event, EventLog};
use portable_pty::{Child, CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock as StdRwLock};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneSize {
    pub cols: u16,
    pub rows: u16,
}

impl PaneSize {
    pub fn validate(self) -> Result<Self, String> {
        if self.cols == 0 || self.rows == 0 {
            return Err("cols and rows must be >= 1".to_string());
        }
        Ok(self)
    }
}

pub struct Pane {
    pub id: Uuid,
    pub name: Option<String>,
    pub group: Arc<StdRwLock<Option<String>>>,
    pub master: Arc<tokio::sync::Mutex<Box<dyn MasterPty + Send>>>,
    pub writer: Arc<tokio::sync::Mutex<Box<dyn Write + Send>>>,
    /// Child process — wrapped in Arc<Mutex<Option>> so both the read loop
    /// and the delete handler can race to take() and reap it exactly once.
    pub child: Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>,
    /// PID stored at creation time so delete can kill without locking.
    pub child_pid: Option<u32>,
    pub parser: Arc<RwLock<vt100::Parser>>,
    pub event_log: Arc<RwLock<EventLog>>,
    pub broadcast_tx: broadcast::Sender<Arc<Event>>,
    pub size: StdRwLock<PaneSize>,
    /// Set to true when the PTY read loop exits (shell exited or was killed).
    pub terminated: Arc<AtomicBool>,
    /// Epoch millis of the last pane activity. Updated on input writes and PTY output.
    pub last_activity_ms: Arc<AtomicU64>,
}

impl Pane {
    pub fn size(&self) -> PaneSize {
        *self.size.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_size(&self, size: PaneSize) {
        *self.size.write().unwrap_or_else(|e| e.into_inner()) = size;
    }

    pub fn group(&self) -> Option<String> {
        self.group.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_group(&self, group: Option<String>) {
        *self.group.write().unwrap_or_else(|e| e.into_inner()) = group;
    }

    pub fn kill_process(&self, signal: libc::c_int) -> Result<(), String> {
        kill_child_process(self.child_pid, signal)
    }

    pub fn take_child(&self) -> Option<Box<dyn Child + Send + Sync>> {
        take_child(&self.child)
    }
}

pub fn create_pane(
    size: PaneSize,
    shell: Option<String>,
    name: Option<String>,
    group: Option<String>,
    max_events: usize,
) -> Result<Arc<Pane>, String> {
    let size = size.validate()?;
    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("openpty failed: {e}"))?;

    let shell_path = shell.unwrap_or_else(|| "/bin/bash".to_string());
    let mut cmd = CommandBuilder::new(&shell_path);
    cmd.env("TERM", "xterm-256color");

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("spawn_command failed: {e}"))?;

    // CRITICAL: drop slave so reader gets EOF when child exits
    drop(pair.slave);

    let child_pid = child.process_id();

    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("take_writer failed: {e}"))?;

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("try_clone_reader failed: {e}"))?;

    let parser = Arc::new(RwLock::new(vt100::Parser::new(size.rows, size.cols, 0)));
    let event_log = Arc::new(RwLock::new(EventLog::with_max_events(max_events)));
    let (broadcast_tx, _) = broadcast::channel::<Arc<Event>>(4096);
    let terminated = Arc::new(AtomicBool::new(false));

    let id = Uuid::new_v4();
    let master = Arc::new(tokio::sync::Mutex::new(pair.master));
    let writer = Arc::new(tokio::sync::Mutex::new(writer));

    let last_activity_ms = Arc::new(AtomicU64::new(now_ms()));
    let pane = Arc::new(Pane {
        id,
        name,
        group: Arc::new(StdRwLock::new(group)),
        master,
        writer,
        child: Arc::new(Mutex::new(Some(child))),
        child_pid,
        parser: parser.clone(),
        event_log: event_log.clone(),
        broadcast_tx: broadcast_tx.clone(),
        size: StdRwLock::new(size),
        terminated: terminated.clone(),
        last_activity_ms: last_activity_ms.clone(),
    });

    let child_arc = Arc::clone(&pane.child);
    spawn_pty_read_loop(
        reader,
        parser,
        event_log,
        broadcast_tx,
        terminated,
        child_arc,
        last_activity_ms,
    );

    Ok(pane)
}

fn spawn_pty_read_loop(
    mut reader: Box<dyn Read + Send>,
    parser: Arc<RwLock<vt100::Parser>>,
    event_log: Arc<RwLock<EventLog>>,
    tx: broadcast::Sender<Arc<Event>>,
    terminated: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>,
    last_activity_ms: Arc<AtomicU64>,
) {
    tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = buf[..n].to_vec();
                    last_activity_ms.store(now_ms(), Ordering::Relaxed);
                    {
                        parser.blocking_write().process(&data);
                    }
                    let event = {
                        let mut log = event_log.blocking_write();
                        let seq = log.push(data.clone());
                        Arc::new(Event {
                            seq,
                            timestamp_ms: now_ms(),
                            data,
                        })
                    };
                    let _ = tx.send(event);
                }
                Err(error) => match classify_read_error(&error) {
                    PtyReadAction::Continue => continue,
                    PtyReadAction::BreakSilently => break,
                    PtyReadAction::BreakWithLog => {
                        eprintln!("PTY read error: {error}");
                        break;
                    }
                },
            }
        }
        terminated.store(true, Ordering::Relaxed);
        // Reap the child (calls waitpid) to avoid zombie processes.
        // If delete_pane already took it, take() returns None — that's fine.
        drop(take_child(&child));
    });
}

pub async fn resize_pane(pane: &Pane, size: PaneSize) -> Result<(), String> {
    let size = size.validate()?;

    {
        let master = pane.master.lock().await;
        master
            .resize(PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("resize failed: {e}"))?;
    }

    {
        let mut parser = pane.parser.write().await;
        parser.screen_mut().set_size(size.rows, size.cols);
    }

    pane.set_size(size);
    Ok(())
}

fn take_child(
    child: &Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>,
) -> Option<Box<dyn Child + Send + Sync>> {
    child.lock().unwrap_or_else(|e| e.into_inner()).take()
}

fn kill_child_process(child_pid: Option<u32>, signal: libc::c_int) -> Result<(), String> {
    let Some(pid) = child_pid else {
        return Ok(());
    };

    let pid = pid as libc::pid_t;
    if pid <= 0 {
        return Ok(());
    }

    let rc = unsafe { libc::kill(pid, signal) };
    if rc == 0 {
        return Ok(());
    }

    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }

    Err(format!("kill({pid}, {signal}) failed: {error}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PtyReadAction {
    Continue,
    BreakSilently,
    BreakWithLog,
}

fn classify_read_error(error: &io::Error) -> PtyReadAction {
    if error.kind() == io::ErrorKind::Interrupted {
        return PtyReadAction::Continue;
    }

    if error.raw_os_error() == Some(libc::EIO) {
        return PtyReadAction::BreakSilently;
    }

    PtyReadAction::BreakWithLog
}

#[cfg(test)]
mod tests {
    use super::{classify_read_error, PtyReadAction};
    use std::io;

    #[test]
    fn classify_read_error_retries_interrupted_reads() {
        let error = io::Error::new(io::ErrorKind::Interrupted, "signal");
        assert_eq!(classify_read_error(&error), PtyReadAction::Continue);
    }

    #[test]
    fn classify_read_error_treats_eio_as_normal_shutdown() {
        let error = io::Error::from_raw_os_error(libc::EIO);
        assert_eq!(classify_read_error(&error), PtyReadAction::BreakSilently);
    }

    #[test]
    fn classify_read_error_logs_other_failures() {
        let error = io::Error::other("boom");
        assert_eq!(classify_read_error(&error), PtyReadAction::BreakWithLog);
    }
}
