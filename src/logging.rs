use anyhow::{Context, Result};
use std::{
    env,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use tracing::{Level, Metadata};
use tracing_subscriber::{
    EnvFilter, Layer, Registry, filter::filter_fn, fmt, layer::SubscriberExt,
    util::SubscriberInitExt,
};

pub const LOG_DIR_VARIABLE: &str = "MCP_KALI_LOG_DIR";
pub const MAIN_LOG_FILE: &str = "mcp-kali.jsonl";
pub const ERROR_LOG_FILE: &str = "mcp-kali.error.jsonl";

/// Keeps asynchronous writers alive and lets SIGHUP reopen fixed log names.
#[derive(Clone)]
pub struct LoggingHandle {
    inner: Arc<LoggingState>,
}

struct LoggingState {
    configured_dir: Option<PathBuf>,
    file_active: Arc<AtomicBool>,
    main_writer: SharedWriter,
    error_writer: SharedWriter,
    guards: Mutex<WriterGuards>,
}

struct WriterGuards {
    _main: AsyncGuard,
    _error: AsyncGuard,
}

#[derive(Clone)]
struct SharedWriter(Arc<RwLock<AsyncWriter>>);

impl<'writer> fmt::MakeWriter<'writer> for SharedWriter {
    type Writer = AsyncLineWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        self.0
            .read()
            .expect("logging writer lock poisoned")
            .line_writer()
    }
}

#[derive(Clone)]
struct AsyncWriter {
    sender: mpsc::SyncSender<WriterMessage>,
}

struct AsyncLineWriter {
    sender: mpsc::SyncSender<WriterMessage>,
    buffer: Vec<u8>,
}

enum WriterMessage {
    Line(Vec<u8>),
    Flush(mpsc::Sender<io::Result<()>>),
    Shutdown,
}

struct AsyncGuard {
    sender: mpsc::SyncSender<WriterMessage>,
    worker: Option<thread::JoinHandle<()>>,
}

impl AsyncWriter {
    fn line_writer(&self) -> AsyncLineWriter {
        AsyncLineWriter {
            sender: self.sender.clone(),
            buffer: Vec::new(),
        }
    }

    fn flush_blocking(&self) -> io::Result<()> {
        let (sender, receiver) = mpsc::channel();
        self.sender
            .send(WriterMessage::Flush(sender))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "logging worker stopped"))?;
        receiver.recv_timeout(Duration::from_secs(2)).map_err(|_| {
            io::Error::new(io::ErrorKind::TimedOut, "logging worker flush timed out")
        })?
    }
}

impl Write for AsyncLineWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.send_buffer()
    }
}

impl AsyncLineWriter {
    fn send_buffer(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let bytes = std::mem::take(&mut self.buffer);
        self.sender
            .send(WriterMessage::Line(bytes))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "logging worker stopped"))
    }
}

impl Drop for AsyncLineWriter {
    fn drop(&mut self) {
        let _ = self.send_buffer();
    }
}

impl Drop for AsyncGuard {
    fn drop(&mut self) {
        let _ = self.sender.send(WriterMessage::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Initializes server logging. A usable configured directory receives split
/// JSONL; otherwise every level is formatted for stdout.
pub fn init() -> Result<LoggingHandle> {
    let configured_dir = env::var_os(LOG_DIR_VARIABLE).map(PathBuf::from);
    let file_active = Arc::new(AtomicBool::new(false));
    let initial = configured_dir
        .as_deref()
        .map(|directory| open_file_writers(directory, file_active.clone()))
        .transpose();
    let (writers, fallback_error) = match initial {
        Ok(Some(writers)) => (writers, None),
        Ok(None) => (sink_writers()?, None),
        Err(error) => (sink_writers()?, Some(error)),
    };
    file_active.store(
        configured_dir.is_some() && fallback_error.is_none(),
        Ordering::Release,
    );
    let main_writer = SharedWriter(Arc::new(RwLock::new(writers.main)));
    let error_writer = SharedWriter(Arc::new(RwLock::new(writers.error)));
    let handle = LoggingHandle {
        inner: Arc::new(LoggingState {
            configured_dir: configured_dir.clone(),
            file_active: file_active.clone(),
            main_writer: main_writer.clone(),
            error_writer: error_writer.clone(),
            guards: Mutex::new(writers.guards),
        }),
    };

    let stdout_active = file_active.clone();
    let main_active = file_active.clone();
    let error_active = file_active;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "mcp_kali=info,tower_http=info".into());
    let stdout_layer = fmt::layer()
        .with_writer(io::stdout)
        .with_filter(filter_fn(move |_| !stdout_active.load(Ordering::Acquire)));
    let main_layer = fmt::layer()
        .json()
        .with_current_span(true)
        .with_span_list(true)
        .with_writer(main_writer)
        .with_filter(filter_fn(move |metadata| {
            main_active.load(Ordering::Acquire) && is_main_level(metadata)
        }));
    let error_layer = fmt::layer()
        .json()
        .with_current_span(true)
        .with_span_list(true)
        .with_writer(error_writer)
        .with_filter(filter_fn(move |metadata| {
            error_active.load(Ordering::Acquire) && is_error_level(metadata)
        }));
    Registry::default()
        .with(filter)
        .with(stdout_layer)
        .with(main_layer)
        .with(error_layer)
        .try_init()
        .context("initialize logging subscriber")?;

    match (configured_dir, fallback_error) {
        (Some(directory), Some(error)) => tracing::warn!(
            log_dir = %directory.display(),
            %error,
            "configured file logging unavailable; using stdout"
        ),
        (None, _) => tracing::info!(
            variable = LOG_DIR_VARIABLE,
            "file logging is not configured; using stdout"
        ),
        _ => {}
    }
    Ok(handle)
}

impl LoggingHandle {
    /// Flushes both asynchronous queues and reopens the fixed filenames. If
    /// either file cannot be reopened, both streams atomically fall back to
    /// stdout until a later SIGHUP succeeds.
    pub fn reopen(&self) -> Result<bool> {
        let Some(directory) = self.inner.configured_dir.as_deref() else {
            return Ok(false);
        };
        self.flush();
        match open_file_writers(directory, self.inner.file_active.clone()) {
            Ok(writers) => {
                self.replace_writers(writers);
                self.inner.file_active.store(true, Ordering::Release);
                Ok(true)
            }
            Err(error) => {
                self.inner.file_active.store(false, Ordering::Release);
                self.replace_writers(sink_writers()?);
                Err(error)
            }
        }
    }

    fn flush(&self) {
        let main = self
            .inner
            .main_writer
            .0
            .read()
            .expect("logging writer lock poisoned")
            .clone();
        let error = self
            .inner
            .error_writer
            .0
            .read()
            .expect("logging writer lock poisoned")
            .clone();
        let _ = main.flush_blocking();
        let _ = error.flush_blocking();
    }

    fn replace_writers(&self, writers: OpenWriters) {
        *self
            .inner
            .main_writer
            .0
            .write()
            .expect("logging writer lock poisoned") = writers.main;
        *self
            .inner
            .error_writer
            .0
            .write()
            .expect("logging writer lock poisoned") = writers.error;
        let old = {
            let mut guards = self
                .inner
                .guards
                .lock()
                .expect("logging guard lock poisoned");
            std::mem::replace(&mut *guards, writers.guards)
        };
        drop(old);
    }
}

impl Drop for LoggingState {
    fn drop(&mut self) {
        let main = self
            .main_writer
            .0
            .read()
            .expect("logging writer lock poisoned")
            .clone();
        let error = self
            .error_writer
            .0
            .read()
            .expect("logging writer lock poisoned")
            .clone();
        let _ = main.flush_blocking();
        let _ = error.flush_blocking();
    }
}

struct OpenWriters {
    main: AsyncWriter,
    error: AsyncWriter,
    guards: WriterGuards,
}

fn open_file_writers(directory: &Path, file_active: Arc<AtomicBool>) -> Result<OpenWriters> {
    let metadata = std::fs::symlink_metadata(directory)
        .with_context(|| format!("inspect logging directory {}", directory.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        anyhow::bail!(
            "logging directory is not a non-symlink directory: {}",
            directory.display()
        );
    }
    let main = open_private_log(&directory.join(MAIN_LOG_FILE))?;
    let error = open_private_log(&directory.join(ERROR_LOG_FILE))?;
    let (main, main_guard) = async_writer(main, Some((file_active.clone(), MAIN_LOG_FILE)))?;
    let (error, error_guard) = async_writer(error, Some((file_active, ERROR_LOG_FILE)))?;
    Ok(OpenWriters {
        main,
        error,
        guards: WriterGuards {
            _main: main_guard,
            _error: error_guard,
        },
    })
}

fn sink_writers() -> Result<OpenWriters> {
    let (main, main_guard) = async_writer(io::sink(), None)?;
    let (error, error_guard) = async_writer(io::sink(), None)?;
    Ok(OpenWriters {
        main,
        error,
        guards: WriterGuards {
            _main: main_guard,
            _error: error_guard,
        },
    })
}

fn async_writer(
    mut destination: impl Write + Send + 'static,
    file_failure: Option<(Arc<AtomicBool>, &'static str)>,
) -> Result<(AsyncWriter, AsyncGuard)> {
    let (sender, receiver) = mpsc::sync_channel(1024);
    let worker = thread::Builder::new()
        .name("mcp-kali-log-writer".into())
        .spawn(move || {
            while let Ok(message) = receiver.recv() {
                match message {
                    WriterMessage::Line(bytes) => {
                        if let Err(error) = destination.write_all(&bytes) {
                            if let Some((active, filename)) = &file_failure {
                                if active.swap(false, Ordering::AcqRel) {
                                    println!(
                                        "mcp-kali could not write {filename}: {error}; using stdout"
                                    );
                                }
                            }
                        }
                    }
                    WriterMessage::Flush(sender) => {
                        let _ = sender.send(destination.flush());
                    }
                    WriterMessage::Shutdown => {
                        let _ = destination.flush();
                        break;
                    }
                }
            }
        })
        .context("spawn logging worker")?;
    Ok((
        AsyncWriter {
            sender: sender.clone(),
        },
        AsyncGuard {
            sender,
            worker: Some(worker),
        },
    ))
}

fn open_private_log(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .with_context(|| format!("open log file {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("set private permissions on {}", path.display()))?;
    }
    Ok(file)
}

fn is_main_level(metadata: &Metadata<'_>) -> bool {
    is_main(*metadata.level())
}

fn is_error_level(metadata: &Metadata<'_>) -> bool {
    is_error(*metadata.level())
}

fn is_main(level: Level) -> bool {
    matches!(level, Level::TRACE | Level::DEBUG | Level::INFO)
}

fn is_error(level: Level) -> bool {
    matches!(level, Level::WARN | Level::ERROR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_exclusively_split() {
        assert!(is_main(Level::INFO));
        assert!(!is_error(Level::INFO));
        assert!(!is_main(Level::WARN));
        assert!(is_error(Level::WARN));
    }

    #[test]
    fn opens_fixed_private_log_files() {
        let directory = tempfile::tempdir().unwrap();
        let writers = open_file_writers(directory.path(), Arc::new(AtomicBool::new(true))).unwrap();
        drop(writers);
        assert!(directory.path().join(MAIN_LOG_FILE).is_file());
        assert!(directory.path().join(ERROR_LOG_FILE).is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(directory.path().join(MAIN_LOG_FILE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn rejects_missing_and_symlink_directories() {
        let directory = tempfile::tempdir().unwrap();
        assert!(
            open_file_writers(
                &directory.path().join("missing"),
                Arc::new(AtomicBool::new(true))
            )
            .is_err()
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(directory.path(), directory.path().join("link")).unwrap();
            assert!(
                open_file_writers(
                    &directory.path().join("link"),
                    Arc::new(AtomicBool::new(true))
                )
                .is_err()
            );
        }
    }
}
