//! Bringing whisper.cpp's server in: the official build where there is one,
//! and the source built on this machine on macOS.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use super::process::{Kept, last_said};
use super::{Catalog, Fetching, Pinned, Step};

const ENGINE: &str = "engine";
const SERVER: &str = "whisper-server";
const VERSION: &str = "version";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    LinuxX64,
    LinuxArm64,
    MacOs,
    Other,
}

/// What of the machine bringing the server in depends on.
#[derive(Debug, Clone)]
pub struct Host {
    pub platform: Platform,
    /// A CMake the machine already has.
    pub cmake: Option<PathBuf>,
    /// Tells whether the compiler's tools are there, as `xcode-select -p`
    /// does. The compiler itself is not asked: without the tools, macOS
    /// answers it with a window offering to install them.
    pub xcode_select: PathBuf,
}

impl Host {
    pub fn current() -> Self {
        let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => Platform::LinuxX64,
            ("linux", "aarch64") => Platform::LinuxArm64,
            ("macos", _) => Platform::MacOs,
            _ => Platform::Other,
        };
        let cmake = std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join("cmake"))
                .find(|cmake| cmake.is_file())
        });
        Self {
            platform,
            cmake,
            xcode_select: "/usr/bin/xcode-select".into(),
        }
    }

    pub(super) fn builds(&self) -> bool {
        self.platform == Platform::MacOs
    }
}

pub(super) fn server(dir: &Path) -> PathBuf {
    dir.join(ENGINE).join(SERVER)
}

pub(super) fn libraries(dir: &Path) -> OsString {
    dir.join(ENGINE).into_os_string()
}

fn up_to_date(dir: &Path, catalog: &Catalog) -> bool {
    server(dir).is_file()
        && std::fs::read_to_string(dir.join(ENGINE).join(VERSION))
            .is_ok_and(|v| v == catalog.version)
}

/// What is to be fetched before the server can be run, if anything.
fn wanted<'a>(dir: &Path, host: &Host, catalog: &'a Catalog) -> Result<Vec<&'a Pinned>> {
    if up_to_date(dir, catalog) {
        return Ok(Vec::new());
    }
    Ok(match host.platform {
        Platform::LinuxX64 => vec![&catalog.linux_x64],
        Platform::LinuxArm64 => vec![&catalog.linux_arm64],
        Platform::MacOs => {
            let cmake = host.cmake.is_none().then_some(&catalog.cmake_macos);
            std::iter::once(&catalog.source).chain(cmake).collect()
        }
        Platform::Other => bail!("whisper.cpp is not brought in on this OS"),
    })
}

/// How much [`install`] downloads.
pub(super) fn bytes_to_fetch(dir: &Path, host: &Host, catalog: &Catalog) -> u64 {
    wanted(dir, host, catalog)
        .unwrap_or_default()
        .iter()
        .map(|p| p.size)
        .sum()
}

/// Bring the server of `catalog`'s version into `dir`, unless it is there.
pub(super) async fn install(
    dir: &Path,
    host: &Host,
    catalog: &Catalog,
    fetching: &Fetching,
) -> Result<()> {
    let wanted = wanted(dir, host, catalog)?;
    if wanted.is_empty() {
        return Ok(());
    }
    if host.builds() {
        developer_tools(host).await?;
    }
    let work = dir.join("engine.new");
    remove(&work)?;
    std::fs::create_dir_all(&work)?;
    let installed = bring(&work, host, catalog, fetching).await;
    let engine = dir.join(ENGINE);
    let moved = installed.and_then(|made| {
        std::fs::write(made.join(VERSION), &catalog.version)?;
        remove(&engine)?;
        std::fs::rename(made, &engine)?;
        Ok(())
    });
    remove(&work)?;
    moved
}

/// Make the server's directory under `work`.
async fn bring(
    work: &Path,
    host: &Host,
    catalog: &Catalog,
    fetching: &Fetching,
) -> Result<PathBuf> {
    match host.platform {
        Platform::LinuxX64 => unpack(work, "build", &catalog.linux_x64, fetching).await,
        Platform::LinuxArm64 => unpack(work, "build", &catalog.linux_arm64, fetching).await,
        Platform::MacOs => build(work, host, catalog, fetching).await,
        Platform::Other => bail!("whisper.cpp is not brought in on this OS"),
    }
}

/// Fetch the archive `pinned` and open it as `work/name`, without the one
/// directory everything in it is under.
async fn unpack(work: &Path, name: &str, pinned: &Pinned, fetching: &Fetching) -> Result<PathBuf> {
    let archive = work.join(format!("{name}.tar.gz"));
    fetching.fetch(Step::Engine, pinned, &archive).await?;
    let into = work.join(name);
    std::fs::create_dir(&into)?;
    let args: [&std::ffi::OsStr; 5] = [
        "-xzf".as_ref(),
        archive.as_ref(),
        "-C".as_ref(),
        into.as_ref(),
        "--strip-components=1".as_ref(),
    ];
    run("tar", &args, &work.join("tar.log")).await?;
    std::fs::remove_file(archive)?;
    Ok(into)
}

async fn developer_tools(host: &Host) -> Result<()> {
    let found = tokio::process::Command::new(&host.xcode_select)
        .arg("-p")
        .output()
        .await
        .is_ok_and(|output| output.status.success());
    ensure!(
        found,
        "Xcode Command Line Tools are needed to build whisper.cpp: run `xcode-select --install`"
    );
    Ok(())
}

async fn build(
    work: &Path,
    host: &Host,
    catalog: &Catalog,
    fetching: &Fetching,
) -> Result<PathBuf> {
    let source = unpack(work, "source", &catalog.source, fetching).await?;
    let cmake = match &host.cmake {
        Some(cmake) => cmake.clone(),
        None => unpack(work, "cmake", &catalog.cmake_macos, fetching)
            .await?
            .join("CMake.app/Contents/bin/cmake"),
    };
    fetching.report(Step::Building, 0, 0);
    let built = work.join("built");
    let log = work.join("cmake.log");
    let configure: [&std::ffi::OsStr; 7] = [
        "-S".as_ref(),
        source.as_ref(),
        "-B".as_ref(),
        built.as_ref(),
        "-DCMAKE_BUILD_TYPE=Release".as_ref(),
        "-DBUILD_SHARED_LIBS=OFF".as_ref(),
        "-DWHISPER_BUILD_TESTS=OFF".as_ref(),
    ];
    run(&cmake, &configure, &log).await?;
    let jobs = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .to_string();
    let compile: [&std::ffi::OsStr; 6] = [
        "--build".as_ref(),
        built.as_ref(),
        "--target".as_ref(),
        SERVER.as_ref(),
        "--parallel".as_ref(),
        jobs.as_ref(),
    ];
    run(&cmake, &compile, &log).await?;
    let made = work.join("made");
    std::fs::create_dir(&made)?;
    std::fs::copy(built.join("bin").join(SERVER), made.join(SERVER))
        .context("the build made no server")?;
    std::fs::copy(source.join("LICENSE"), made.join("LICENSE"))?;
    Ok(made)
}

async fn run<S: AsRef<std::ffi::OsStr>>(
    program: impl AsRef<std::ffi::OsStr>,
    args: &[S],
    log: &Path,
) -> Result<()> {
    let program = program.as_ref();
    let name = Path::new(program)
        .file_name()
        .unwrap_or(program)
        .to_string_lossy()
        .into_owned();
    let mut kept = Kept::start(program, args, &[], log)?;
    ensure!(kept.wait().await?, "{name} failed: {}", last_said(log));
    Ok(())
}

fn remove(dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("removing {}", dir.display()))
        }
        _ => Ok(()),
    }
}
