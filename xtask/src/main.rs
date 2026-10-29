//! Workspace automation, run via `cargo xtask <command>` (see `.cargo/config.toml`).

use std::{
    env,
    io::BufRead,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
};

use clap::{Parser, Subcommand, ValueEnum};

type Result<T> = std::result::Result<T, String>;

#[derive(Parser)]
#[command(about = "Silicate workspace tasks")]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Windows Explorer thumbnail provider (libs/silica-thumbs)
    Thumbs {
        #[command(subcommand)]
        command: ThumbsTask,
    },
}

#[derive(Subcommand)]
enum ThumbsTask {
    /// Build the thumbnail DLL into target/silica-thumbs/
    Build {
        #[arg(long, value_enum, default_value_t = Arch::X64)]
        arch: Arch,
    },
    /// Build the DLL and register it with Explorer, replacing any previous registration
    Register {
        #[arg(long, value_enum, default_value_t = Arch::X64)]
        arch: Arch,
        /// Register machine-wide instead of for the current user (requires an elevated shell)
        #[arg(long)]
        machine: bool,
        /// Restart Explorer so it releases the old DLL and picks up the new one
        #[arg(long)]
        restart_explorer: bool,
    },
    /// Unregister the previously built DLL
    Unregister {
        #[arg(long, value_enum, default_value_t = Arch::X64)]
        arch: Arch,
        /// Remove the machine-wide registration instead (requires an elevated shell)
        #[arg(long)]
        machine: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Arch {
    X64,
    X86,
    Arm64,
}

impl Arch {
    fn triple(self) -> &'static str {
        match self {
            Arch::X64 => "x86_64-pc-windows-msvc",
            Arch::X86 => "i686-pc-windows-msvc",
            Arch::Arm64 => "aarch64-pc-windows-msvc",
        }
    }

    /// File name of the installed DLL. Kept stable so existing registrations stay valid.
    fn dll_name(self) -> String {
        let suffix = match self {
            Arch::X64 => "x64",
            Arch::X86 => "x86",
            Arch::Arm64 => "arm64",
        };
        format!("win_procreate_thumbs_{suffix}.dll")
    }
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Task::Thumbs { command } => match command {
            ThumbsTask::Build { arch } => thumbs_build(arch).map(|dll| {
                println!("Built {}", dll.display());
            }),
            ThumbsTask::Register {
                arch,
                machine,
                restart_explorer,
            } => thumbs_register(arch, machine, restart_explorer),
            ThumbsTask::Unregister { arch, machine } => thumbs_unregister(arch, machine),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Builds the DLL and copies it to `target/silica-thumbs/<dll_name>`, returning that path.
fn thumbs_build(arch: Arch) -> Result<PathBuf> {
    let artifact = cargo_build_dll(arch)?;
    let installed = installed_dll_path(&artifact, arch)?;
    copy_dll(&artifact, &installed)?;
    Ok(installed)
}

fn thumbs_register(arch: Arch, machine: bool, restart_explorer: bool) -> Result<()> {
    let artifact = cargo_build_dll(arch)?;
    let installed = installed_dll_path(&artifact, arch)?;

    // Drop the old registration first so Explorer never points at a half-copied DLL.
    if installed.exists() {
        let _ = regsvr32(&installed, machine, false);
    }

    if restart_explorer {
        stop_explorer();
    }
    let result = copy_dll(&artifact, &installed).and_then(|()| regsvr32(&installed, machine, true));
    if restart_explorer {
        start_explorer();
    }
    result?;

    println!("Registered {}", installed.display());
    Ok(())
}

fn thumbs_unregister(arch: Arch, machine: bool) -> Result<()> {
    let installed = workspace_root()
        .join("target")
        .join("silica-thumbs")
        .join(arch.dll_name());
    if !installed.exists() {
        return Err(format!(
            "{} does not exist; nothing to unregister",
            installed.display()
        ));
    }
    regsvr32(&installed, machine, false)?;
    println!("Unregistered {}", installed.display());
    Ok(())
}

/// Runs `cargo build` for the DLL and returns the path cargo reports for it.
fn cargo_build_dll(arch: Arch) -> Result<PathBuf> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut child = Command::new(cargo)
        .current_dir(workspace_root())
        .args(["build", "--profile", "release-dll", "-p", "silica-thumbs"])
        .args(["--target", arch.triple()])
        .arg("--message-format=json-render-diagnostics")
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run cargo: {e}"))?;

    let mut dll = None;
    let stdout = child.stdout.take().expect("stdout is piped");
    for line in std::io::BufReader::new(stdout).lines() {
        let line = line.map_err(|e| e.to_string())?;
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if msg["reason"] != "compiler-artifact" || msg["target"]["name"] != "silica_thumbs" {
            continue;
        }
        let filenames = msg["filenames"].as_array().into_iter().flatten();
        if let Some(path) = filenames
            .filter_map(|f| f.as_str())
            .find(|f| f.ends_with(".dll"))
        {
            dll = Some(PathBuf::from(path));
        }
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("cargo build failed".into());
    }
    dll.ok_or_else(|| "cargo did not report a DLL for silica-thumbs".into())
}

/// `<target-dir>/silica-thumbs/<dll_name>`, where the artifact is at
/// `<target-dir>/<triple>/<profile>/silica_thumbs.dll`.
fn installed_dll_path(artifact: &Path, arch: Arch) -> Result<PathBuf> {
    let target_dir = artifact
        .ancestors()
        .nth(3)
        .ok_or_else(|| format!("unexpected artifact path {}", artifact.display()))?;
    Ok(target_dir.join("silica-thumbs").join(arch.dll_name()))
}

fn copy_dll(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to.parent().expect("path has a parent")).map_err(|e| e.to_string())?;
    std::fs::copy(from, to).map(drop).map_err(|e| {
        format!(
            "failed to copy DLL to {}: {e}\n\
             If Explorer still has the old DLL loaded, rerun with --restart-explorer.",
            to.display()
        )
    })
}

/// Per-user registration goes through `DllInstall` (`/n /i:user`), which needs no elevation.
fn regsvr32(dll: &Path, machine: bool, install: bool) -> Result<()> {
    let mut cmd = Command::new("regsvr32");
    cmd.arg("/s");
    if !install {
        cmd.arg("/u");
    }
    if !machine {
        cmd.args(["/n", "/i:user"]);
    }
    let status = cmd
        .arg(dll)
        .status()
        .map_err(|e| format!("failed to run regsvr32: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        let action = if install { "registration" } else { "unregistration" };
        let hint = if machine {
            " (machine-wide changes require an elevated shell)"
        } else {
            ""
        };
        Err(format!("regsvr32 {action} failed with {status}{hint}"))
    }
}

fn stop_explorer() {
    let _ = Command::new("taskkill")
        .args(["/f", "/im", "explorer.exe"])
        .stdout(Stdio::null())
        .status();
}

/// Windows usually restarts the shell on its own after it is killed; launching
/// `explorer.exe` while it is already running would just open a new window.
fn start_explorer() {
    let running = Command::new("tasklist")
        .args(["/nh", "/fi", "imagename eq explorer.exe"])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains("explorer.exe"));
    if !running {
        let _ = Command::new("explorer.exe").spawn();
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace root")
        .to_path_buf()
}
