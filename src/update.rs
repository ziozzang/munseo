//! GitHub Releases 자동 업데이트 (ziozzang/sugyeol 방식).
//!
//! 최신 릴리스 조회 → 버전 비교 → 플랫폼 에셋 다운로드 → SHA256SUMS 검증 → 교체 → 다시 실행.
//! - macOS: `HWP2PDF_<ver>_macos_universal.zip` 을 풀어 `.app` 번들을 통째로 바꾼다.
//!   앱이 끝난 뒤 셸 스크립트가 번들을 바꾸고 다시 연다.
//! - Windows: `HWP2PDF_<ver>_windows_x86_64.exe` 를 받아, 실행 중인 exe 를 `.old` 로 이름을 바꾸고
//!   (Windows 는 실행 중인 파일의 이름 변경을 허용) 새 exe 를 그 자리에 둔 뒤 실행한다.

use std::cmp::Ordering;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const REPOSITORY: &str = "ziozzang/munseo";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 3600);
const USER_AGENT: &str = "hwp2pdf-selfupdate";
#[cfg(target_os = "macos")]
const BUNDLE_ID: &str = "net.jioh.hwp2pdf";

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub body: Option<String>,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

impl Release {
    pub fn version(&self) -> &str {
        self.tag_name.strip_prefix('v').unwrap_or(&self.tag_name)
    }

    pub fn is_newer(&self) -> bool {
        compare_versions(self.version(), CURRENT_VERSION) == Ordering::Greater
    }

    pub fn notes(&self) -> &str {
        self.body.as_deref().unwrap_or("").trim()
    }
}

// MARK: 조회

fn agent(timeout: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(timeout)
        .timeout_connect(Some(Duration::from_secs(10)))
        .build()
        .into()
}

pub fn fetch_latest() -> Result<Release, String> {
    // HWP2PDF_UPDATE_API 로 테스트 서버를 가리킬 수 있다.
    let api = std::env::var("HWP2PDF_UPDATE_API").unwrap_or_else(|_| "https://api.github.com".into());
    let url = format!("{api}/repos/{REPOSITORY}/releases/latest");
    let mut resp = agent(Some(Duration::from_secs(15)))
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("릴리스 정보를 가져오지 못했습니다: {e}"))?;
    let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("릴리스 정보를 해석하지 못했습니다: {e}"))
}

/// 숫자 점 표기 비교. 앞의 `v`, 뒤의 `-pre` / `+build` 는 무시하고, 없는 자리는 0 으로 본다.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    fn parts(s: &str) -> Vec<u64> {
        let s = s.strip_prefix('v').unwrap_or(s);
        let core = s.split(['-', '+']).next().unwrap_or("");
        core.split('.').map(|p| p.parse().unwrap_or(0)).collect()
    }
    let (x, y) = (parts(a), parts(b));
    for i in 0..x.len().max(y.len()) {
        let (l, r) = (x.get(i).copied().unwrap_or(0), y.get(i).copied().unwrap_or(0));
        if l != r {
            return l.cmp(&r);
        }
    }
    Ordering::Equal
}

pub fn asset_name(version: &str) -> Option<String> {
    if cfg!(target_os = "macos") {
        Some(format!("HWP2PDF_{version}_macos_universal.zip"))
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Some(format!("HWP2PDF_{version}_windows_x86_64.exe"))
    } else {
        None
    }
}

/// `sha256sum` 형식: "<64자리 hex>  <파일 이름>"
pub fn parse_checksums(text: &str) -> std::collections::HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let (hash, name, rest) = (f.next()?, f.next()?, f.next());
            (rest.is_none() && hash.len() == 64)
                .then(|| (name.trim_start_matches('*').to_string(), hash.to_ascii_lowercase()))
        })
        .collect()
}

// MARK: 확인 주기와 건너뛴 버전 (캐시 파일)

#[derive(Default, Serialize, Deserialize)]
struct CheckState {
    checked_at: u64,
    latest_version: String,
    skipped_version: String,
}

fn state_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches"));
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")));
    base.map(|b| b.join("hwp2pdf").join("update-check.json"))
}

fn load_state() -> CheckState {
    state_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_state(state: &CheckState) {
    let Some(path) = state_path() else { return };
    let Some(dir) = path.parent() else { return };
    let _ = fs::create_dir_all(dir);
    let tmp = path.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_vec(state) {
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 시작할 때 자동 확인을 할지: 끄는 환경 변수가 없고, 마지막 확인에서 하루가 지났을 때.
pub fn automatic_check_due() -> bool {
    if std::env::var_os("HWP2PDF_NO_UPDATE_CHECK").is_some() || asset_name(CURRENT_VERSION).is_none() {
        return false;
    }
    now().saturating_sub(load_state().checked_at) >= CHECK_INTERVAL.as_secs()
}

pub fn record_check(release: &Release) {
    let mut state = load_state();
    state.checked_at = now();
    state.latest_version = release.version().to_string();
    save_state(&state);
}

pub fn is_skipped(release: &Release) -> bool {
    load_state().skipped_version == release.version()
}

pub fn skip(release: &Release) {
    let mut state = load_state();
    state.skipped_version = release.version().to_string();
    save_state(&state);
}

// MARK: 설치

/// 받아서 검증하고, 앱이 끝나면 교체(`relaunch` 면 재실행까지)되도록 준비한다.
/// `Ok` 가 돌아오면 호출한 쪽은 바로 앱을 끝내야 한다.
pub fn install(release: &Release, relaunch: bool, progress: &dyn Fn(f32, String)) -> Result<(), String> {
    let name = asset_name(release.version()).ok_or("이 플랫폼용 업데이트 파일이 없습니다.")?;
    let asset = find_asset(release, &name)?;
    let sums_asset = find_asset(release, "SHA256SUMS")?;

    progress(0.0, "체크섬 가져오는 중".into());
    let sums_text = agent(Some(Duration::from_secs(30)))
        .get(&sums_asset.browser_download_url)
        .header("User-Agent", USER_AGENT)
        .call()
        .and_then(|mut r| r.body_mut().read_to_string())
        .map_err(|e| format!("SHA256SUMS 를 받지 못했습니다: {e}"))?;
    let expected = parse_checksums(&sums_text)
        .remove(&name)
        .ok_or_else(|| format!("SHA256SUMS 에 {name} 항목이 없습니다."))?;

    platform::install(release, asset, &expected, relaunch, progress)
}

fn find_asset<'a>(release: &'a Release, name: &str) -> Result<&'a Asset, String> {
    release
        .assets
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| format!("릴리스에 {name} 파일이 없습니다."))
}

/// 내려받으면서 SHA-256 을 계산하고, 기대값과 다르면 파일을 지운다.
fn download_verified(asset: &Asset, dest: &Path, expected: &str, progress: &dyn Fn(f32, String)) -> Result<(), String> {
    let mut resp = agent(None)
        .get(&asset.browser_download_url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("업데이트를 받지 못했습니다: {e}"))?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok()?.parse::<u64>().ok())
        .unwrap_or(asset.size)
        .max(1);

    let mut reader = resp.body_mut().as_reader();
    let mut file = fs::File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let (mut received, mut last_percent) = (0u64, u64::MAX);
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("다운로드 중 오류: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        received += n as u64;
        let percent = received * 100 / total;
        if percent != last_percent {
            last_percent = percent;
            progress((received as f32 / total as f32).min(1.0), format!("다운로드 중 {}%", percent.min(100)));
        }
    }
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);

    let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if !actual.eq_ignore_ascii_case(expected) {
        let _ = fs::remove_file(dest);
        return Err(format!("{} 체크섬이 맞지 않습니다.\n받은 값: {actual}\n기대 값: {expected}", asset.name));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::process::Command;

    pub fn install(release: &Release, asset: &Asset, expected: &str, relaunch: bool, progress: &dyn Fn(f32, String)) -> Result<(), String> {
        let exe = std::env::current_exe().and_then(|p| p.canonicalize()).map_err(|e| e.to_string())?;
        // …/HWP2PDF.app/Contents/MacOS/hwp2pdf
        let bundle = exe
            .ancestors()
            .nth(3)
            .filter(|p| p.extension().is_some_and(|e| e == "app"))
            .ok_or("앱 번들(.app)로 실행할 때만 업데이트할 수 있습니다.")?
            .to_path_buf();
        let parent = bundle.parent().ok_or("앱 위치를 알 수 없습니다.")?;

        // 앱과 같은 볼륨에 작업 폴더를 두어 마지막 이동이 이름 변경으로 끝나게 한다.
        let work = parent.join(format!(".hwp2pdf-update-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        fs::create_dir(&work).map_err(|_| format!("{} 폴더에 쓸 권한이 없어 업데이트할 수 없습니다.", parent.display()))?;
        let result = (|| {
            let zip = work.join(&asset.name);
            download_verified(asset, &zip, expected, progress)?;

            progress(1.0, "압축 푸는 중".into());
            let ok = Command::new("/usr/bin/ditto").arg("-x").arg("-k").arg(&zip).arg(&work).status();
            if !ok.is_ok_and(|s| s.success()) {
                return Err("내려받은 압축 파일을 풀지 못했습니다.".to_string());
            }
            let new_app = fs::read_dir(&work)
                .map_err(|e| e.to_string())?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .find(|p| p.extension().is_some_and(|e| e == "app"))
                .ok_or("압축 파일 안에 앱이 없습니다.")?;
            let plist = new_app.join("Contents/Info.plist");
            if plist_value(&plist, "CFBundleIdentifier").as_deref() != Some(BUNDLE_ID)
                || plist_value(&plist, "CFBundleShortVersionString").as_deref() != Some(release.version())
            {
                return Err("내려받은 앱의 번들 ID 또는 버전이 맞지 않습니다.".to_string());
            }
            spawn_swap(&bundle, &new_app, &work, relaunch)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&work);
        }
        result
    }

    fn plist_value(plist: &Path, key: &str) -> Option<String> {
        let out = Command::new("/usr/libexec/PlistBuddy").arg("-c").arg(format!("Print :{key}")).arg(plist).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// 이 프로세스가 끝나기를 기다렸다가 번들을 바꾸고 다시 여는 셸 스크립트를 띄운다.
    fn spawn_swap(target: &Path, new_app: &Path, work: &Path, relaunch: bool) -> Result<(), String> {
        let script = r#"
while kill -0 "$PID" 2>/dev/null; do sleep 0.2; done
rm -rf "$TARGET.old"
if mv "$TARGET" "$TARGET.old" && mv "$NEW" "$TARGET"; then
  rm -rf "$TARGET.old"
else
  [ -e "$TARGET" ] || mv "$TARGET.old" "$TARGET"
fi
rm -rf "$WORK"
xattr -dr com.apple.quarantine "$TARGET" 2>/dev/null
[ "$RELAUNCH" = 1 ] && open "$TARGET"
exit 0
"#;
        Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("PID", std::process::id().to_string())
            .env("TARGET", target)
            .env("NEW", new_app)
            .env("WORK", work)
            .env("RELAUNCH", if relaunch { "1" } else { "0" })
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("업데이트 설치를 시작하지 못했습니다: {e}"))
    }
}

#[cfg(windows)]
mod platform {
    use super::*;

    pub fn install(_release: &Release, asset: &Asset, expected: &str, relaunch: bool, progress: &dyn Fn(f32, String)) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let new = exe.with_extension("exe.new");
        let old = exe.with_extension("exe.old");
        download_verified(asset, &new, expected, progress)
            .map_err(|e| if e.contains("os error 5") { format!("{} 폴더에 쓸 권한이 없어 업데이트할 수 없습니다.", exe.parent().unwrap_or(&exe).display()) } else { e })?;

        progress(1.0, "설치 중".into());
        let _ = fs::remove_file(&old);
        fs::rename(&exe, &old).map_err(|e| format!("실행 파일을 바꾸지 못했습니다: {e}"))?;
        if let Err(e) = fs::rename(&new, &exe) {
            let _ = fs::rename(&old, &exe);
            return Err(format!("실행 파일을 바꾸지 못했습니다: {e}"));
        }
        if !relaunch {
            return Ok(());
        }
        std::process::Command::new(&exe)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("새 버전을 실행하지 못했습니다: {e}"))
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use super::*;

    pub fn install(_: &Release, _: &Asset, _: &str, _: bool, _: &dyn Fn(f32, String)) -> Result<(), String> {
        Err("이 플랫폼은 자동 업데이트를 지원하지 않습니다.".into())
    }
}

/// Windows 에서 지난 업데이트가 남긴 `.exe.old` 를 지운다.
pub fn cleanup_previous() {
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        let _ = fs::remove_file(exe.with_extension("exe.old"));
    }
}

// MARK: 명령줄

/// `hwp2pdf update [--check]`
pub fn run_cli(check_only: bool) -> std::process::ExitCode {
    use std::process::ExitCode;
    let release = match fetch_latest() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    record_check(&release);
    println!("현재 버전: {CURRENT_VERSION}\n최신 버전: {}", release.version());
    if !release.is_newer() {
        println!("최신 버전을 사용 중입니다.");
        return ExitCode::SUCCESS;
    }
    if check_only {
        println!("새 버전이 있습니다. `hwp2pdf update` 로 업데이트하세요.");
        return ExitCode::SUCCESS;
    }
    // 단계가 바뀔 때와 10% 단위로만 출력한다.
    let last = std::cell::Cell::new(u32::MAX);
    let report = |p: f32, stage: String| {
        let step = if stage.starts_with("다운로드") { (p * 10.0) as u32 } else { 100 + stage.len() as u32 };
        if step != last.replace(step) {
            eprintln!("{stage}");
        }
    };
    match install(&release, false, &report) {
        Ok(()) => {
            println!("{} → {} 업데이트를 설치했습니다.", CURRENT_VERSION, release.version());
            if !release.notes().is_empty() {
                println!("--- v{} ---\n{}", release.version(), release.notes());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("업데이트 실패: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(compare_versions("1.0.0", "1.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.2.0", "1.10.0"), Ordering::Less);
        assert_eq!(compare_versions("v2.0.0-beta", "1.9.9"), Ordering::Greater);
        assert_eq!(compare_versions("0.1.1", "0.1.0+3"), Ordering::Greater);
    }

    #[test]
    fn checksums() {
        let h = "a".repeat(64);
        let sums = parse_checksums(&format!("{h}  HWP2PDF_1.0.0_macos_universal.zip\nbad line\n{h} *x.exe\n"));
        assert_eq!(sums.get("HWP2PDF_1.0.0_macos_universal.zip"), Some(&h));
        assert_eq!(sums.get("x.exe"), Some(&h));
        assert_eq!(sums.len(), 2);
    }
}
