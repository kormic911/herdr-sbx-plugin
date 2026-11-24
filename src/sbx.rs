use regex::RegexSet;
use std::{
    env,
    fs::File,
    io::{self, Read, Seek},
    process::{Command, ExitStatus, Stdio},
    sync::LazyLock,
    thread,
    time::{Duration, Instant},
};

pub struct Failure {
    pub kind: &'static str,
    pub message: String,
    pub output: String,
}

impl Failure {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            output: String::new(),
        }
    }
}

impl From<io::Error> for Failure {
    fn from(e: io::Error) -> Self {
        Self::new("unknown", e.to_string())
    }
}

pub fn classify(output: &str) -> &'static str {
    static PATTERNS: LazyLock<RegexSet> = LazyLock::new(|| {
        RegexSet::new([
        r"(?i)sandboxd|daemon (?:is )?not running|(?:cannot|could not|can't|unable to) connect to (?:the )?daemon|is the daemon running",
        r"(?i)not logged in|sbx login|unauthori[sz]ed|authentication (?:required|failed)|\b401\b",
        r"(?i)permission denied|forbidden|operation not permitted|\b403\b",
        r"(?i)no such sandbox|not found|does not exist|unknown sandbox|no sandbox (?:named|with|called)",
        r"(?i)already exists|already in use|in use by|is in use|active session|has (?:an? )?(?:open|active)|conflict",
        r"(?i)connection refused|network is unreachable|no such host|timed out|timeout|ECONNRESET|ECONNREFUSED|EAI_AGAIN|proxy",
    ]).unwrap()
    });
    const KINDS: [&str; 6] = [
        "daemon",
        "authentication",
        "permission",
        "not-found",
        "conflict",
        "network",
    ];
    PATTERNS
        .matches(output)
        .into_iter()
        .next()
        .map_or("unknown", |i| KINDS[i])
}

const LIMIT: u64 = 16 * 1024 * 1024;

fn read_output(file: &mut File) -> io::Result<String> {
    file.rewind()?;
    let mut bytes = Vec::new();
    file.take(LIMIT).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub struct Output {
    pub status: ExitStatus,
    pub stdout: String,
    pub combined: String,
}

pub fn run(bin: &str, args: &[&str]) -> Result<Output, Failure> {
    let timeout = env::var("HERDR_SBX_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n > 0.)
        .map_or(Duration::from_secs(120), |n| {
            Duration::from_millis(n.max(1.) as u64)
        });
    run_cancellable(bin, args, timeout, || false)
}

pub fn run_cancellable(
    bin: &str,
    args: &[&str],
    timeout: Duration,
    cancelled: impl Fn() -> bool,
) -> Result<Output, Failure> {
    if cancelled() {
        return Err(Failure::new("cancelled", "cancelled"));
    }
    // Separate spool files avoid pipe deadlocks on large output and inherited handles.
    let mut out = tempfile::tempfile()?;
    let mut err = tempfile::tempfile()?;
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(err.try_clone()?)
        .spawn()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Failure::new(
                "startup",
                format!("The sbx CLI was not found (looked for \"{bin}\"). Install Docker Sandboxes or set config.sbxBin."),
            ),
            _ => Failure::new("unknown", format!("Could not run {bin}: {e}")),
        })?;
    let start = Instant::now();
    let result = (|| loop {
        if cancelled() {
            return Err(Failure::new("cancelled", "cancelled"));
        }
        let status = child.try_wait()?;
        if out.metadata()?.len() > LIMIT || err.metadata()?.len() > LIMIT {
            return Err(Failure::new(
                "unknown",
                "sbx output exceeds the capture limit.",
            ));
        }
        if let Some(status) = status {
            return Ok(status);
        }
        if start.elapsed() >= timeout {
            let seconds = timeout.as_secs_f64();
            let command = args[0];
            return Err(Failure::new(
                "daemon",
                format!("{bin} {command} did not finish within {seconds}s and was killed."),
            ));
        }
        thread::sleep(Duration::from_millis(10).min(timeout.saturating_sub(start.elapsed())));
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let stdout = read_output(&mut out)?;
    let combined = format!("{stdout}{}", read_output(&mut err)?);
    match result {
        Ok(status) => Ok(Output {
            status,
            stdout,
            combined,
        }),
        Err(mut failure) => {
            failure.output = combined;
            Err(failure)
        }
    }
}
