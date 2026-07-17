use crate::models::{Job, JobState, OutputPage, SubmitJob};
use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncSeekExt},
    process::Command,
    sync::{Mutex, Notify, Semaphore},
    time::timeout,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

const MAX_OUTPUT_PAGE: usize = 1024 * 1024;

#[derive(Clone)]
pub struct Scheduler {
    inner: Arc<Inner>,
}

struct Inner {
    root: PathBuf,
    jobs: Mutex<HashMap<Uuid, Job>>,
    cancellations: Mutex<HashMap<Uuid, CancellationToken>>,
    permits: Arc<Semaphore>,
    notify: Notify,
    default_timeout: u64,
    max_concurrency: usize,
    webhook_client: reqwest::Client,
}

impl Scheduler {
    pub async fn open(root: PathBuf, max_concurrency: usize, default_timeout: u64) -> Result<Self> {
        if max_concurrency == 0 {
            bail!("max_concurrency must be greater than zero");
        }
        fs::create_dir_all(&root)
            .await
            .context("create job state directory")?;
        let scheduler = Self {
            inner: Arc::new(Inner {
                root,
                jobs: Mutex::new(HashMap::new()),
                cancellations: Mutex::new(HashMap::new()),
                permits: Arc::new(Semaphore::new(max_concurrency)),
                notify: Notify::new(),
                default_timeout,
                max_concurrency,
                webhook_client: reqwest::Client::new(),
            }),
        };
        scheduler.load().await?;
        let dispatcher = scheduler.clone();
        tokio::spawn(async move { dispatcher.dispatch().await });
        scheduler.inner.notify.notify_one();
        Ok(scheduler)
    }

    async fn load(&self) -> Result<()> {
        let mut entries = fs::read_dir(&self.inner.root).await?;
        let mut jobs = self.inner.jobs.lock().await;
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            let metadata = entry.path().join("job.json");
            let Ok(bytes) = fs::read(&metadata).await else {
                continue;
            };
            let Ok(mut job) = serde_json::from_slice::<Job>(&bytes) else {
                warn!(path = %metadata.display(), "ignoring invalid job metadata");
                continue;
            };
            job.argv = match fs::read(entry.path().join("command.json")).await {
                Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
                Err(_) => Vec::new(),
            };
            let mut changed = false;
            if job.argv.is_empty() && job.state == JobState::Queued {
                job.state = JobState::Interrupted;
                job.finished_at = Some(Utc::now());
                job.error = Some("private execution specification is missing".into());
                changed = true;
            }
            if job.state == JobState::Running {
                job.state = JobState::Interrupted;
                job.finished_at = Some(Utc::now());
                job.error = Some("server restarted while job was running".into());
                changed = true;
            }
            if changed {
                persist_at(&self.inner.root, &job).await?;
            }
            jobs.insert(job.id, job);
        }
        Ok(())
    }

    pub async fn submit(&self, request: SubmitJob) -> Result<Job> {
        if request.argv.is_empty() || request.argv[0].is_empty() {
            bail!("argv must contain an executable");
        }
        let timeout_seconds = request
            .timeout_seconds
            .unwrap_or(self.inner.default_timeout);
        if timeout_seconds == 0 || timeout_seconds > 7 * 24 * 60 * 60 {
            bail!("timeout_seconds must be between 1 and 604800");
        }
        if let Some(url) = &request.webhook_url {
            let parsed = reqwest::Url::parse(url).context("invalid webhook_url")?;
            if parsed.scheme() != "https"
                && !parsed
                    .host_str()
                    .is_some_and(|h| h == "127.0.0.1" || h == "localhost")
            {
                bail!("webhook_url must use HTTPS (HTTP is allowed only for localhost)");
            }
        }
        let job = Job {
            id: Uuid::new_v4(),
            tool: request.tool.unwrap_or_else(|| request.argv[0].clone()),
            argv: request.argv,
            state: JobState::Queued,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            timeout_seconds,
            return_code: None,
            error: None,
            webhook_url: request.webhook_url,
        };
        persist_at(&self.inner.root, &job).await?;
        self.inner.jobs.lock().await.insert(job.id, job.clone());
        self.inner.notify.notify_one();
        Ok(job)
    }

    pub async fn get(&self, id: Uuid) -> Option<Job> {
        self.inner.jobs.lock().await.get(&id).cloned()
    }

    pub async fn list(&self) -> Vec<Job> {
        let mut jobs: Vec<_> = self.inner.jobs.lock().await.values().cloned().collect();
        jobs.sort_by_key(|j| std::cmp::Reverse(j.created_at));
        jobs
    }

    pub async fn counts(&self) -> (usize, usize, usize) {
        let jobs = self.inner.jobs.lock().await;
        let queued = jobs
            .values()
            .filter(|j| j.state == JobState::Queued)
            .count();
        let running = jobs
            .values()
            .filter(|j| j.state == JobState::Running)
            .count();
        (queued, running, self.inner.max_concurrency)
    }

    pub async fn cancel(&self, id: Uuid) -> Result<Job> {
        let state = self
            .get(id)
            .await
            .ok_or_else(|| anyhow!("job not found"))?
            .state;
        match state {
            JobState::Queued => {
                let mut jobs = self.inner.jobs.lock().await;
                let job = jobs.get_mut(&id).ok_or_else(|| anyhow!("job not found"))?;
                job.state = JobState::Cancelled;
                job.finished_at = Some(Utc::now());
                persist_at(&self.inner.root, job).await?;
            }
            JobState::Running => self
                .inner
                .cancellations
                .lock()
                .await
                .get(&id)
                .ok_or_else(|| anyhow!("job is transitioning to running; retry cancellation"))?
                .cancel(),
            _ => bail!("job is already terminal"),
        }
        self.get(id).await.ok_or_else(|| anyhow!("job not found"))
    }

    pub async fn output(
        &self,
        id: Uuid,
        stream: &str,
        offset: u64,
        limit: usize,
    ) -> Result<OutputPage> {
        if self.get(id).await.is_none() {
            bail!("job not found");
        }
        if !["stdout", "stderr"].contains(&stream) {
            bail!("stream must be stdout or stderr");
        }
        let limit = limit.clamp(1, MAX_OUTPUT_PAGE);
        let path = self.job_dir(id).join(format!("{stream}.log"));
        let mut file = match fs::File::open(path).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(OutputPage {
                    job_id: id,
                    stream: stream.into(),
                    offset,
                    next_offset: offset,
                    truncated: false,
                    data: String::new(),
                });
            }
            Err(error) => return Err(error.into()),
        };
        let size = file.metadata().await?.len();
        let start = offset.min(size);
        file.seek(std::io::SeekFrom::Start(start)).await?;
        let mut bytes = vec![0; limit];
        let read = file.read(&mut bytes).await?;
        bytes.truncate(read);
        Ok(OutputPage {
            job_id: id,
            stream: stream.into(),
            offset: start,
            next_offset: start + read as u64,
            truncated: start + (read as u64) < size,
            data: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }

    fn job_dir(&self, id: Uuid) -> PathBuf {
        self.inner.root.join(id.to_string())
    }

    async fn dispatch(&self) {
        loop {
            self.inner.notify.notified().await;
            loop {
                let Ok(permit) = self.inner.permits.clone().acquire_owned().await else {
                    return;
                };
                let Some(id) = self.next_queued().await else {
                    drop(permit);
                    break;
                };
                let scheduler = self.clone();
                tokio::spawn(async move {
                    if let Err(error) = scheduler.run(id).await {
                        error!(%id, %error, "job runner failed");
                    }
                    drop(permit);
                    scheduler.inner.notify.notify_one();
                });
            }
        }
    }

    async fn next_queued(&self) -> Option<Uuid> {
        let mut jobs = self.inner.jobs.lock().await;
        let id = jobs
            .values()
            .filter(|j| j.state == JobState::Queued)
            .min_by_key(|j| j.created_at)?
            .id;
        // Reserving as running here prevents dispatching a queued job twice.
        let job = jobs.get_mut(&id)?;
        job.state = JobState::Running;
        job.started_at = Some(Utc::now());
        self.inner
            .cancellations
            .lock()
            .await
            .insert(id, CancellationToken::new());
        if let Err(error) = persist_at(&self.inner.root, job).await {
            error!(%id, %error, "could not persist running state");
            job.state = JobState::Failed;
            job.error = Some(error.to_string());
            return None;
        }
        Some(id)
    }

    async fn run(&self, id: Uuid) -> Result<()> {
        let job = self
            .get(id)
            .await
            .ok_or_else(|| anyhow!("job disappeared"))?;
        let dir = self.job_dir(id);
        let stdout = std::fs::File::create(dir.join("stdout.log"))?;
        let stderr = std::fs::File::create(dir.join("stderr.log"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            stdout.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            stderr.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        let mut command = Command::new(&job.argv[0]);
        command
            .args(&job.argv[1..])
            .stdin(std::process::Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
        }
        let token = self
            .inner
            .cancellations
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow!("job cancellation token disappeared"))?;
        info!(%id, tool = %job.tool, "job started");

        let outcome = match command.spawn() {
            Err(error) => (
                JobState::Failed,
                None,
                Some(format!("failed to start {}: {error}", job.argv[0])),
            ),
            Ok(mut child) => tokio::select! {
                _ = token.cancelled() => {
                    terminate(&mut child).await;
                    (JobState::Cancelled, None, None)
                }
                result = timeout(Duration::from_secs(job.timeout_seconds), child.wait()) => match result {
                    Err(_) => {
                        terminate(&mut child).await;
                        (JobState::TimedOut, None, Some(format!("timed out after {} seconds", job.timeout_seconds)))
                    }
                    Ok(Err(error)) => (JobState::Failed, None, Some(error.to_string())),
                    Ok(Ok(status)) if status.success() => (JobState::Succeeded, status.code(), None),
                    Ok(Ok(status)) => (JobState::Failed, status.code(), None),
                }
            },
        };
        self.inner.cancellations.lock().await.remove(&id);
        let completed = {
            let mut jobs = self.inner.jobs.lock().await;
            let job = jobs
                .get_mut(&id)
                .ok_or_else(|| anyhow!("job disappeared"))?;
            job.state = outcome.0;
            job.return_code = outcome.1;
            job.error = outcome.2;
            job.finished_at = Some(Utc::now());
            persist_at(&self.inner.root, job).await?;
            job.clone()
        };
        info!(%id, state = ?completed.state, "job finished");
        self.send_webhook(&completed).await;
        Ok(())
    }

    async fn send_webhook(&self, job: &Job) {
        let Some(url) = &job.webhook_url else { return };
        match self
            .inner
            .webhook_client
            .post(url)
            .json(job)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                info!(id = %job.id, "webhook delivered")
            }
            Ok(response) => warn!(id = %job.id, status = %response.status(), "webhook rejected"),
            Err(error) => warn!(id = %job.id, %error, "webhook delivery failed"),
        }
    }
}

async fn terminate(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // The child starts its own process group so scanner descendants do not
        // survive cancellation or timeout as orphaned processes.
        unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
        if timeout(Duration::from_secs(5), child.wait()).await.is_err() {
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
            let _ = child.wait().await;
        }
        return;
    }
    let _ = child.kill().await;
}

async fn persist_at(root: &Path, job: &Job) -> Result<()> {
    let dir = root.join(job.id.to_string());
    fs::create_dir_all(&dir).await?;
    let final_path = dir.join("job.json");
    let temporary = dir.join("job.json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(job)?).await?;
    fs::rename(temporary, final_path).await?;
    fs::write(dir.join("command.json"), serde_json::to_vec(&job.argv)?).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).await?;
        fs::set_permissions(dir.join("job.json"), std::fs::Permissions::from_mode(0o600)).await?;
        fs::set_permissions(
            dir.join("command.json"),
            std::fs::Permissions::from_mode(0o600),
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn executes_and_pages_output() {
        let temp = tempfile::tempdir().unwrap();
        let scheduler = Scheduler::open(temp.path().into(), 1, 10).await.unwrap();
        let job = scheduler
            .submit(SubmitJob {
                tool: None,
                argv: vec!["printf".into(), "hello".into()],
                timeout_seconds: None,
                webhook_url: None,
            })
            .await
            .unwrap();
        for _ in 0..100 {
            if scheduler.get(job.id).await.unwrap().state.is_terminal() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            scheduler.get(job.id).await.unwrap().state,
            JobState::Succeeded
        );
        assert_eq!(
            scheduler
                .output(job.id, "stdout", 0, 100)
                .await
                .unwrap()
                .data,
            "hello"
        );
    }

    #[tokio::test]
    async fn queued_job_can_be_cancelled_without_starting() {
        let temp = tempfile::tempdir().unwrap();
        let scheduler = Scheduler::open(temp.path().into(), 1, 10).await.unwrap();
        let first = scheduler
            .submit(SubmitJob {
                tool: None,
                argv: vec!["sleep".into(), "0.2".into()],
                timeout_seconds: None,
                webhook_url: None,
            })
            .await
            .unwrap();
        let second = scheduler
            .submit(SubmitJob {
                tool: None,
                argv: vec!["printf".into(), "must-not-run".into()],
                timeout_seconds: None,
                webhook_url: None,
            })
            .await
            .unwrap();
        for _ in 0..100 {
            if scheduler.get(first.id).await.unwrap().state == JobState::Running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(
            scheduler.get(second.id).await.unwrap().state,
            JobState::Queued
        );
        scheduler.cancel(second.id).await.unwrap();
        assert_eq!(
            scheduler.get(second.id).await.unwrap().state,
            JobState::Cancelled
        );
    }
}
