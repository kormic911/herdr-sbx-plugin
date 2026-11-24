use crate::sbx::Failure;
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime},
};

pub fn now() -> String {
    DateTime::<Utc>::from(SystemTime::now()).to_rfc3339_opts(SecondsFormat::Millis, true)
}
pub fn nonce() -> Result<String, Failure> {
    let mut bytes = [0u8; 6];
    getrandom::fill(&mut bytes).map_err(|e| Failure::new("startup", e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn stem(id: &str) -> String {
    // JS replaces one UTF-16 code unit at a time, then truncates to 64.
    let label: String = id
        .encode_utf16()
        .take(64)
        .map(|u| match u8::try_from(u) {
            Ok(b) if b.is_ascii_alphanumeric() || b"._-".contains(&b) => b as char,
            _ => '_',
        })
        .collect();
    let hash = format!("{:x}", Sha256::digest(id.as_bytes()));
    format!("{label}-{}", &hash[..10])
}

#[cfg(unix)]
fn process(pid: u32) -> (bool, Option<String>) {
    if pid > i32::MAX as u32 {
        return (false, None);
    }
    // Signal zero tests existence without signalling the process.
    let alive = unsafe { libc::kill(pid as i32, 0) } == 0
        || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    let token = fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| {
            s.rsplit_once(')')?
                .1
                .split_whitespace()
                .nth(19)
                .map(|n| format!("linux:{n}"))
        });
    #[cfg(target_os = "macos")]
    let token = token.or_else(|| {
        let out = std::process::Command::new("ps")
            .args(["-o", "lstart=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        out.status.success().then(|| {
            format!(
                "ps:{}",
                String::from_utf8_lossy(&out.stdout)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join("_")
            )
        })
    });
    (alive, token)
}
#[cfg(windows)]
fn process(pid: u32) -> (bool, Option<String>) {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, FILETIME},
        System::Threading::{
            GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    };
    // Query-only handle. Access denial is treated as a potentially live owner.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return (
                io::Error::last_os_error().raw_os_error() != Some(ERROR_INVALID_PARAMETER as i32),
                None,
            );
        }
        let mut creation: FILETIME = std::mem::zeroed();
        let (mut exit, mut kernel, mut user) = (creation, creation, creation);
        let mut status = 0;
        let alive = GetExitCodeProcess(h, &mut status) == 0 || status == 259;
        let token = (GetProcessTimes(h, &mut creation, &mut exit, &mut kernel, &mut user) != 0)
            .then(|| {
                let ticks =
                    ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
                format!("win32:{}", ticks + 504911232000000000)
            });
        CloseHandle(h);
        (alive, token)
    }
}
pub fn live_owner(pid: &Value, token: &Value) -> bool {
    let Some(pid) = pid
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
    else {
        return false;
    };
    let (alive, current) = process(pid);
    alive
        && !matches!((token.as_str(), current.as_deref()), (Some(a), Some(b)) if a != "-" && a != b)
}

pub fn own_token() -> Option<String> {
    process(std::process::id()).1
}

pub struct Lock {
    path: PathBuf,
    owner: String,
}
impl Drop for Lock {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path).ok().as_deref() == Some(&self.owner) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn stale(path: &Path, text: &str, wait: Duration) -> bool {
    let mut words = text.split_whitespace();
    let age = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .unwrap_or_default();
    let Some(pid) = words
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .filter(|p| *p > 0)
    else {
        return age > wait;
    };
    let (alive, token) = process(pid);
    !alive || matches!((words.next(),token.as_deref()),(Some(a),Some(b)) if a!="-" && a!=b)
}
pub fn lock(path: PathBuf) -> Result<Lock, Failure> {
    let wait = Duration::from_millis(
        env::var("HERDR_SBX_LOCK_WAIT_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(5000),
    );
    let start = Instant::now();
    let owner = format!(
        "{} {}\n",
        std::process::id(),
        own_token().as_deref().unwrap_or("-")
    );
    loop {
        match fs::File::create_new(&path) {
            Ok(mut f) => {
                if let Err(e) = f.write_all(owner.as_bytes()) {
                    drop(f);
                    let _ = fs::remove_file(&path);
                    return Err(e.into());
                }
                return Ok(Lock { path, owner });
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        if let Ok(seen) = fs::read_to_string(&path) {
            if stale(&path, &seen, wait) {
                let guard = PathBuf::from(format!("{}.reclaim", path.display()));
                if let Ok(mut f) = fs::File::create_new(&guard) {
                    let _ = writeln!(f, "{}", std::process::id());
                    drop(f);
                    if fs::read_to_string(&path).ok().as_ref() == Some(&seen)
                        && stale(&path, &seen, wait)
                    {
                        let _ = fs::remove_file(&path);
                    }
                    let _ = fs::remove_file(guard);
                } else if let Ok(text) = fs::read_to_string(&guard) {
                    if stale(&guard, &text, wait) {
                        let _ = fs::remove_file(guard);
                    }
                }
            }
        }
        if start.elapsed() >= wait {
            return Err(Failure::new(
                "conflict",
                format!("Mapping is locked: {}", path.display()),
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

pub struct Mapping<'a> {
    pub dir: &'a Path,
    pub pane: &'a str,
    pub name: &'a str,
}
impl Mapping<'_> {
    pub fn change(
        &self,
        f: impl FnOnce(Option<Value>) -> Result<Value, Failure>,
    ) -> Result<Value, Failure> {
        self.change_with_names(&[self.name], f)
    }

    pub fn change_with_names(
        &self,
        names: &[&str],
        f: impl FnOnce(Option<Value>) -> Result<Value, Failure>,
    ) -> Result<Value, Failure> {
        self.transaction(names, None, |old, file| {
            let row = f(old)?;
            save(file, self.pane, row)
        })
    }

    pub fn transaction<T>(
        &self,
        names: &[&str],
        destination: Option<&str>,
        f: impl FnOnce(Option<Value>, &Path) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        let dir = self.dir.join("panes");
        fs::create_dir_all(&dir)?;
        let mut locks: Vec<_> = names
            .iter()
            .map(|name| dir.join(format!("sandbox-{}.lock", stem(name))))
            .collect();
        locks.sort();
        locks.dedup();
        let pane = stem(self.pane);
        let _sandboxes = locks.into_iter().map(lock).collect::<Result<Vec<_>, _>>()?;
        let mut panes = vec![pane.clone()];
        if let Some(id) = destination {
            panes.push(stem(id));
        }
        panes.sort();
        panes.dedup();
        let _panes = panes
            .into_iter()
            .map(|p| lock(dir.join(format!("{p}.json.lock"))))
            .collect::<Result<Vec<_>, _>>()?;
        let file = dir.join(format!("{pane}.json"));
        let old = match fs::read(&file) {
            Ok(data) => {
                let row: Value = serde_json::from_slice(&data)
                    .map_err(|e| Failure::new("startup", e.to_string()))?;
                if row["version"] != 1 || row["paneId"] != self.pane {
                    return Err(Failure::new("conflict", "Unexpected pane mapping."));
                }
                Some(row)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        f(old, &file)
    }

    pub fn patch(&self, patch: Value) -> Result<(), Failure> {
        self.change(|old| {
            let mut old =
                old.ok_or_else(|| Failure::new("conflict", "The mapping disappeared."))?;
            if old["sandboxName"] != self.name
                || old["bridgePid"] != std::process::id()
                || !old["deletingPid"].is_null()
            {
                return Err(Failure::new(
                    "conflict",
                    "The bridge no longer owns this mapping.",
                ));
            }
            old.as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            Ok(old)
        })?;
        Ok(())
    }
}

pub fn mapping_path(dir: &Path, pane: &str) -> PathBuf {
    dir.join("panes").join(format!("{}.json", stem(pane)))
}

pub fn save(file: &Path, pane: &str, mut row: Value) -> Result<Value, Failure> {
    row["version"] = json!(1);
    row["paneId"] = json!(pane);
    row["revision"] = json!(nonce()?);
    row["updatedAt"] = json!(now());
    let mut tmp = tempfile::NamedTempFile::new_in(file.parent().unwrap())?;
    serde_json::to_writer(&mut tmp, &row).map_err(|e| Failure::new("startup", e.to_string()))?;
    tmp.flush()?;
    tmp.persist(file).map_err(|e| e.error)?;
    Ok(row)
}
