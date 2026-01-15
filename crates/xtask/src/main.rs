use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context as _};
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use dialoguer::{theme::ColorfulTheme, Confirm, Input, MultiSelect, Select};
use directories::BaseDirs;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

#[derive(Parser)]
#[command(version, about = "ForgeFFI 构建工具")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Menu,
    Build(BuildArgs),
    Zig(ZigArgs),
}

#[derive(Parser, Clone)]
struct ZigArgs {
    #[arg(long, default_value = "0.12.0")]
    version: String,
}

#[derive(Parser, Clone)]
struct BuildArgs {
    #[arg(long)]
    target: Option<String>,

    #[arg(long, default_value = "release")]
    profile: BuildProfile,

    #[arg(long, default_value = "module-ffi")]
    mode: BuildMode,

    #[arg(long, value_delimiter = ',', num_args = 0..)]
    modules: Vec<Module>,

    #[arg(long, value_delimiter = ',', num_args = 0..)]
    features: Vec<String>,

    #[arg(long, default_value = "cdylib")]
    artifact: ArtifactKind,

    #[arg(long, default_value = "0.12.0")]
    zig_version: String,

    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    zigbuild: bool,

    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    headers: bool,

    #[arg(long)]
    dist_dir: Option<PathBuf>,
}

#[derive(Copy, Clone, Debug, ValueEnum, Eq, PartialEq)]
enum BuildProfile {
    Debug,
    Release,
}

impl BuildProfile {
    fn as_flag(self) -> Option<&'static str> {
        match self {
            BuildProfile::Debug => None,
            BuildProfile::Release => Some("--release"),
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum, Eq, PartialEq)]
enum BuildMode {
    ModuleRust,
    ModuleFfi,
    AggregateRust,
    AggregateFfi,
}

impl fmt::Display for BuildMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BuildMode::ModuleRust => "module-rust",
            BuildMode::ModuleFfi => "module-ffi",
            BuildMode::AggregateRust => "aggregate-rust",
            BuildMode::AggregateFfi => "aggregate-ffi",
        };
        f.write_str(s)
    }
}

#[derive(Copy, Clone, Debug, ValueEnum, Eq, PartialEq, Ord, PartialOrd)]
enum Module {
    Net,
    Fs,
    Sys,
}

impl Module {
    fn rust_pkg(self) -> &'static str {
        match self {
            Module::Net => "forgeffi-net",
            Module::Fs => "forgeffi-fs",
            Module::Sys => "forgeffi-sys",
        }
    }

    fn ffi_pkg(self) -> &'static str {
        match self {
            Module::Net => "forgeffi-net-ffi",
            Module::Fs => "forgeffi-fs-ffi",
            Module::Sys => "forgeffi-sys-ffi",
        }
    }

}

#[derive(Copy, Clone, Debug, ValueEnum, Eq, PartialEq)]
enum ArtifactKind {
    Cdylib,
    Staticlib,
}

impl ArtifactKind {
    fn as_str(self) -> &'static str {
        match self {
            ArtifactKind::Cdylib => "cdylib",
            ArtifactKind::Staticlib => "staticlib",
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Menu => menu(),
        Commands::Build(args) => build(args),
        Commands::Zig(args) => {
            let zig = ensure_zig(&args.version)?;
            println!("{}", zig.display());
            Ok(())
        }
    }
}

fn menu() -> anyhow::Result<()> {
    let theme = ColorfulTheme::default();

    let profiles = [BuildProfile::Debug, BuildProfile::Release];
    let profile_idx = Select::with_theme(&theme)
        .with_prompt("选择构建 Profile")
        .items(&["debug", "release"])
        .default(1)
        .interact()?;
    let profile = profiles[profile_idx];

    let modes = [
        BuildMode::ModuleRust,
        BuildMode::ModuleFfi,
        BuildMode::AggregateRust,
        BuildMode::AggregateFfi,
    ];
    let mode_labels = ["模块 Rust", "模块 FFI", "聚合 Rust", "聚合 FFI"];
    let mode_idx = Select::with_theme(&theme)
        .with_prompt("选择构建模式")
        .items(&mode_labels)
        .default(1)
        .interact()?;
    let mode = modes[mode_idx];

    let artifact = match mode {
        BuildMode::ModuleFfi | BuildMode::AggregateFfi => {
            let artifacts = [ArtifactKind::Cdylib, ArtifactKind::Staticlib];
            let artifact_idx = Select::with_theme(&theme)
                .with_prompt("选择产物类型")
                .items(&["动态库(cdylib)", "静态库(staticlib)"])
                .default(0)
                .interact()?;
            artifacts[artifact_idx]
        }
        BuildMode::ModuleRust | BuildMode::AggregateRust => ArtifactKind::Cdylib,
    };

    let (modules, features) = match mode {
        BuildMode::ModuleRust | BuildMode::ModuleFfi => {
            let items = ["net", "fs", "sys"];
            let defaults = vec![true, false, false];
            let selected = MultiSelect::with_theme(&theme)
                .with_prompt("选择模块")
                .items(&items)
                .defaults(&defaults)
                .interact()?;
            let mut modules = Vec::with_capacity(selected.len());
            for idx in selected {
                modules.push(match idx {
                    0 => Module::Net,
                    1 => Module::Fs,
                    2 => Module::Sys,
                    _ => unreachable!(),
                });
            }
            (modules, Vec::new())
        }
        BuildMode::AggregateRust | BuildMode::AggregateFfi => {
            let items = ["net", "fs", "sys", "full"];
            let selected = Select::with_theme(&theme)
                .with_prompt("选择聚合 features")
                .items(&items)
                .default(3)
                .interact()?;
            let feature = items[selected].to_string();
            (Vec::new(), vec![feature])
        }
    };

    let zigbuild = Confirm::with_theme(&theme)
        .with_prompt("使用 cargo-zigbuild 进行交叉编译")
        .default(true)
        .interact()?;

    let zig_version = if zigbuild {
        Input::with_theme(&theme)
            .with_prompt("Zig 版本")
            .default("0.12.0".to_string())
            .interact_text()?
    } else {
        "0.12.0".to_string()
    };

    let host = host_target_triple()?;

    let mut targets = Vec::with_capacity(common_targets().len() + 1);
    targets.push(host.clone());
    targets.extend(common_targets());
    targets = unique_targets(targets);

    let mut target_items = Vec::with_capacity(targets.len() + 1);
    target_items.push("all（全部）".to_string());
    target_items.extend(targets);

    let default_target_idx = target_items
        .iter()
        .position(|t| t == &host)
        .unwrap_or(0);

    let target_idx = Select::with_theme(&theme)
        .with_prompt("选择目标平台 (target triple)")
        .items(&target_items)
        .default(default_target_idx)
        .interact()?;

    let all_selected = target_idx == 0;
    let selected_targets = if all_selected {
        target_items
            .iter()
            .skip(1)
            .cloned()
            .collect::<Vec<String>>()
    } else {
        vec![target_items[target_idx].clone()]
    };

    let headers = match mode {
        BuildMode::ModuleFfi | BuildMode::AggregateFfi => {
            Confirm::with_theme(&theme)
                .with_prompt("生成 C 头文件")
                .default(true)
                .interact()?
        }
        _ => false,
    };

    let dist_dir = Some(PathBuf::from("dist"));

    let workspace_root = workspace_root()?;
    let mut failures = Vec::new();

    for original_target in selected_targets {
        if let Some(reason) = skip_target_reason(&host, &original_target, all_selected) {
            println!("提示: 跳过 target={original_target}（{reason}）");
            continue;
        }

        let effective_zigbuild = if zigbuild
            && original_target.contains("windows-msvc")
            && original_target != host
        {
            let mapped = map_windows_msvc_target_for_zigbuild(&original_target);
            let can_map = mapped.is_some();

            let items = if can_map {
                vec![
                    "保持 MSVC target（将自动关闭 zigbuild）",
                    "切换到 zigbuild 支持的 target（GNU/GNU-LLVM）",
                ]
            } else {
                vec!["保持 MSVC target（将自动关闭 zigbuild）"]
            };

            let choice = Select::with_theme(&theme)
                .with_prompt("检测到 Windows MSVC target，zigbuild 可能不兼容")
                .items(&items)
                .default(0)
                .interact()?;

            if can_map && choice == 1 {
                let mapped = mapped.ok_or_else(|| anyhow!("无法映射 target"))?;
                println!(
                    "提示: 为使用 zigbuild，target 已从 {original_target} 切换为 {mapped}"
                );
                run_one_build(
                    &workspace_root,
                    BuildArgs {
                        target: Some(mapped.to_string()),
                        profile,
                        mode,
                        modules: modules.clone(),
                        features: features.clone(),
                        artifact,
                        zig_version: zig_version.clone(),
                        zigbuild: true,
                        headers,
                        dist_dir: dist_dir.clone(),
                    },
                )
                .map_err(|e| failures.push((original_target.clone(), e)))
                .ok();
                continue;
            }

            println!(
                "提示: 将使用 MSVC toolchain 构建，已关闭 zigbuild（target={original_target}）"
            );
            false
        } else if zigbuild && original_target.contains("windows-msvc") {
            false
        } else {
            zigbuild
        };

        run_one_build(
            &workspace_root,
            BuildArgs {
                target: Some(original_target.clone()),
                profile,
                mode,
                modules: modules.clone(),
                features: features.clone(),
                artifact,
                zig_version: zig_version.clone(),
                zigbuild: effective_zigbuild,
                headers,
                dist_dir: dist_dir.clone(),
            },
        )
        .map_err(|e| failures.push((original_target.clone(), e)))
        .ok();
    }

    if failures.is_empty() {
        Ok(())
    } else {
        let mut msg = String::from("部分 target 构建失败:\n");
        for (t, e) in failures {
            msg.push_str(&format!("- {t}: {e:#}\n"));
        }
        bail!(msg)
    }
}

fn run_one_build(_workspace_root: &Path, args: BuildArgs) -> anyhow::Result<()> {
    build(args)
}

fn skip_target_reason(host: &str, target: &str, all_selected: bool) -> Option<String> {
    let host_is_macos = host.contains("apple-darwin");
    let target_is_apple = target.contains("apple-");
    if target_is_apple && !host_is_macos {
        return Some("当前 host 不是 macOS".to_string());
    }

    if target.contains("-linux-android") && !has_android_ndk() {
        return Some("缺少 Android NDK（请设置 ANDROID_NDK_HOME/ANDROID_NDK_ROOT 等）".to_string());
    }

    if all_selected && target.contains("windows-msvc") && target != host {
        return Some("all 模式默认跳过非本机 MSVC 交叉目标".to_string());
    }

    None
}

fn has_android_ndk() -> bool {
    const KEYS: [&str; 4] = ["ANDROID_NDK_HOME", "ANDROID_NDK_ROOT", "NDK_HOME", "NDK_ROOT"];
    KEYS.iter().any(|k| {
        std::env::var_os(k)
            .map(PathBuf::from)
            .is_some_and(|p| p.is_dir())
    })
}

fn unique_targets(mut targets: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::<String>::new();
    targets.retain(|t| seen.insert(t.clone()));
    targets
}

fn build(mut args: BuildArgs) -> anyhow::Result<()> {
    let workspace_root = workspace_root()?;

    if args.target.is_none() {
        args.target = Some(host_target_triple()?);
    }
    let target = args
        .target
        .clone()
        .ok_or_else(|| anyhow!("target 不能为空"))?;

    let host = host_target_triple()?;
    if args.zigbuild && target.contains("windows-msvc") {
        if target == host {
            println!("提示: 当前为本机 MSVC target，使用普通 cargo build（关闭 zigbuild）：{target}");
            args.zigbuild = false;
        } else if let Some(mapped) = map_windows_msvc_target_for_zigbuild(&target) {
            println!("提示: 为使用 zigbuild，target 已从 {target} 切换为 {mapped}");
            args.target = Some(mapped.to_string());
        } else {
            bail!("cargo-zigbuild 不支持该 Windows MSVC target: {target}");
        }
    }

    let target = args
        .target
        .clone()
        .ok_or_else(|| anyhow!("target 不能为空"))?;

    let dist_dir = args
        .dist_dir
        .clone()
        .unwrap_or_else(|| workspace_root.join("dist"));
    fs::create_dir_all(&dist_dir).context("创建 dist 目录失败")?;

    let zig_path = if args.zigbuild {
        ensure_cargo_subcommand("zigbuild")?;
        Some(ensure_zig(&args.zig_version)?)
    } else {
        None
    };

    ensure_rust_target(&target)?;

    let pkgs = resolve_packages(&args)?;
    for pkg in pkgs {
        let (cmd_name, mut cmd) = if args.zigbuild {
            let mut c = Command::new("cargo");
            c.arg("zigbuild");
            ("cargo zigbuild", c)
        } else {
            let mut c = Command::new("cargo");
            c.arg("build");
            ("cargo build", c)
        };

        cmd.current_dir(&workspace_root);
        if let Some(p) = &zig_path {
            cmd.env("ZIG", p);
        }
        cmd.arg("-p").arg(pkg);
        cmd.arg("--target").arg(&target);
        if let Some(flag) = args.profile.as_flag() {
            cmd.arg(flag);
        }
        if !args.features.is_empty() {
            cmd.arg("--features").arg(args.features.join(","));
        }

        run_checked(cmd_name, &mut cmd)?;

        if is_ffi_pkg(pkg) {
            copy_artifact_to_dist(
                &workspace_root,
                &dist_dir,
                pkg,
                &target,
                args.profile,
                args.artifact,
            )?;

            if args.headers {
                generate_c_header_to_dist(&workspace_root, &dist_dir, pkg, &target, args.profile)?;
            }

            build_c_example_netif_list_if_applicable(
                &workspace_root,
                &dist_dir,
                pkg,
                &target,
                args.profile,
                args.artifact,
                &args.zig_version,
            )?;
        }
    }

    Ok(())
}

fn build_c_example_netif_list_if_applicable(
    workspace_root: &Path,
    dist_dir: &Path,
    pkg: &str,
    target: &str,
    profile: BuildProfile,
    artifact: ArtifactKind,
    zig_version: &str,
) -> anyhow::Result<()> {
    if pkg != "forgeffi-net-ffi" && pkg != "forgeffi-ffi" {
        return Ok(());
    }

    let src = workspace_root
        .join("examples")
        .join("c")
        .join("netif_list.c");
    if !src.is_file() {
        return Ok(());
    }

    let zig = ensure_zig(zig_version)?;

    let bin_dir = dist_dir
        .join(target)
        .join(profile_dir_name(profile))
        .join("examples");
    fs::create_dir_all(&bin_dir).context("创建 examples 目录失败")?;

    let exe_name = if target.contains("windows") {
        "netif_list.exe"
    } else {
        "netif_list"
    };
    let exe_path = bin_dir.join(exe_name);

    let mut cmd = Command::new(&zig);
    cmd.arg("cc");
    cmd.arg("-std=c11");

    if let Some(zig_target) = zig_target_from_rust_target(target) {
        cmd.arg("-target").arg(zig_target);
    }

    match profile {
        BuildProfile::Debug => {
            cmd.arg("-O0");
            cmd.arg("-g");
        }
        BuildProfile::Release => {
            cmd.arg("-O2");
        }
    }
    cmd.arg(&src);
    cmd.arg("-o").arg(&exe_path);

    let effective_artifact = match artifact {
        ArtifactKind::Cdylib => {
            if has_cdylib(dist_dir, target, profile, pkg) {
                ArtifactKind::Cdylib
            } else {
                ArtifactKind::Staticlib
            }
        }
        ArtifactKind::Staticlib => ArtifactKind::Staticlib,
    };

    match effective_artifact {
        ArtifactKind::Cdylib => {
            if !target.contains("windows") {
                cmd.arg("-ldl");
            }
        }
        ArtifactKind::Staticlib => {
            cmd.arg("-DFORGEFFI_STATIC=1");
            let include_dir = dist_dir
                .join(target)
                .join(profile_dir_name(profile))
                .join(pkg)
                .join("include");
            cmd.arg("-I").arg(&include_dir);

            let staticlib_dir = dist_dir
                .join(target)
                .join(profile_dir_name(profile))
                .join(pkg)
                .join("staticlib");
            let staticlib_file = staticlib_filename(pkg, target);
            let staticlib_path = staticlib_dir.join(staticlib_file);
            cmd.arg(&staticlib_path);

            if !target.contains("windows") {
                cmd.arg("-lunwind");
            }
        }
    }

    run_checked("zig cc (examples/c/netif_list.c)", &mut cmd)?;
    println!("dist: {}", exe_path.display());

    if effective_artifact == ArtifactKind::Cdylib {
        copy_runtime_dylib_if_present(dist_dir, &bin_dir, pkg, target, profile)?;
    }
    Ok(())
}

fn has_cdylib(dist_dir: &Path, target: &str, profile: BuildProfile, pkg: &str) -> bool {
    let lib_basename = pkg.replace('-', "_");
    let lib_file = if target.contains("windows") {
        format!("{lib_basename}.dll")
    } else if target.contains("apple-darwin") {
        format!("lib{lib_basename}.dylib")
    } else {
        format!("lib{lib_basename}.so")
    };

    dist_dir
        .join(target)
        .join(profile_dir_name(profile))
        .join(pkg)
        .join("cdylib")
        .join(lib_file)
        .is_file()
}

fn zig_target_from_rust_target(rust_target: &str) -> Option<String> {
    let mut it = rust_target.split('-');
    let arch = it.next()?;
    let _vendor = it.next()?;
    let os = it.next()?;
    let env = it.next();

    let os = if os == "darwin" { "macos" } else { os };
    let mut out = String::new();
    out.push_str(arch);
    out.push('-');
    out.push_str(os);

    if let Some(env) = env {
        out.push('-');
        out.push_str(env);
    }

    Some(out)
}

fn staticlib_filename(pkg: &str, target: &str) -> String {
    let lib_basename = pkg.replace('-', "_");
    if target.contains("windows") {
        format!("{lib_basename}.lib")
    } else {
        format!("lib{lib_basename}.a")
    }
}

fn profile_dir_name(profile: BuildProfile) -> &'static str {
    match profile {
        BuildProfile::Debug => "debug",
        BuildProfile::Release => "release",
    }
}

fn copy_runtime_dylib_if_present(
    dist_dir: &Path,
    bin_dir: &Path,
    pkg: &str,
    target: &str,
    profile: BuildProfile,
) -> anyhow::Result<()> {
    let lib_basename = pkg.replace('-', "_");
    let lib_file = if target.contains("windows") {
        format!("{lib_basename}.dll")
    } else if target.contains("apple-darwin") {
        format!("lib{lib_basename}.dylib")
    } else {
        format!("lib{lib_basename}.so")
    };

    let candidate = dist_dir
        .join(target)
        .join(profile_dir_name(profile))
        .join(pkg)
        .join("cdylib")
        .join(&lib_file);
    if !candidate.is_file() {
        return Ok(());
    }

    let dst = bin_dir.join(&lib_file);
    fs::copy(&candidate, &dst).with_context(|| {
        format!(
            "复制运行时动态库失败: {} -> {}",
            candidate.display(),
            dst.display()
        )
    })?;
    println!("dist: {}", dst.display());
    Ok(())
}

fn resolve_packages(args: &BuildArgs) -> anyhow::Result<Vec<&'static str>> {
    match args.mode {
        BuildMode::ModuleRust => {
            let modules = normalize_modules(&args.modules);
            Ok(modules.into_iter().map(Module::rust_pkg).collect())
        }
        BuildMode::ModuleFfi => {
            let modules = normalize_modules(&args.modules);
            Ok(modules.into_iter().map(Module::ffi_pkg).collect())
        }
        BuildMode::AggregateRust => Ok(vec!["forgeffi"]),
        BuildMode::AggregateFfi => Ok(vec!["forgeffi-ffi"]),
    }
}

fn normalize_modules(modules: &[Module]) -> Vec<Module> {
    if modules.is_empty() {
        vec![Module::Net]
    } else {
        let mut s = BTreeSet::new();
        for m in modules {
            s.insert(*m);
        }
        s.into_iter().collect()
    }
}

fn is_ffi_pkg(pkg: &str) -> bool {
    pkg.ends_with("-ffi")
}

fn copy_artifact_to_dist(
    workspace_root: &Path,
    dist_dir: &Path,
    pkg: &str,
    target: &str,
    profile: BuildProfile,
    kind: ArtifactKind,
) -> anyhow::Result<()> {
    let out_dir = match profile {
        BuildProfile::Debug => workspace_root.join("target").join(target).join("debug"),
        BuildProfile::Release => workspace_root.join("target").join(target).join("release"),
    };

    let lib_name = pkg.replace('-', "_");
    let (src, effective_kind) = match find_artifact_path(&out_dir, &lib_name, target, kind) {
        Ok(p) => (p, kind),
        Err(e) => {
            if kind == ArtifactKind::Cdylib {
                let fallback = find_artifact_path(&out_dir, &lib_name, target, ArtifactKind::Staticlib)
                    .with_context(|| {
                        format!("未找到产物: pkg={pkg} kind={} / fallback=staticlib: {e:#}", kind.as_str())
                    })?;
                println!(
                    "提示: target={target} 未生成动态库，已改为输出静态库"
                );
                (fallback, ArtifactKind::Staticlib)
            } else {
                return Err(e).with_context(|| {
                    format!("未找到产物: pkg={pkg} kind={}", kind.as_str())
                });
            }
        }
    };

    let dst_dir = dist_dir
        .join(target)
        .join(match profile {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        })
        .join(pkg)
        .join(effective_kind.as_str());
    fs::create_dir_all(&dst_dir).context("创建 dist 子目录失败")?;
    let dst = dst_dir.join(
        src.file_name()
            .ok_or_else(|| anyhow!("产物路径缺少文件名"))?,
    );

    fs::copy(&src, &dst).with_context(|| {
        format!("复制产物失败: {} -> {}", src.display(), dst.display())
    })?;

    println!("dist: {}", dst.display());

    if effective_kind == ArtifactKind::Cdylib && target.contains("windows") {
        let import_libs = find_windows_import_libs(&out_dir, &lib_name)?;
        if import_libs.is_empty() {
            bail!("未找到 Windows 导入库(.lib/.dll.a)，请检查构建输出: pkg={pkg} target={target}");
        }
        for import_lib in import_libs {
            let dst = dst_dir.join(
                import_lib
                    .file_name()
                    .ok_or_else(|| anyhow!("导入库路径缺少文件名"))?,
            );
            fs::copy(&import_lib, &dst).with_context(|| {
                format!("复制导入库失败: {} -> {}", import_lib.display(), dst.display())
            })?;
            println!("dist: {}", dst.display());
        }
    }
    Ok(())
}

fn find_windows_import_libs(out_dir: &Path, lib_basename: &str) -> anyhow::Result<Vec<PathBuf>> {
    let mut found: std::collections::BTreeMap<String, PathBuf> = std::collections::BTreeMap::new();
    let candidates = [out_dir.to_path_buf(), out_dir.join("deps")];
    for dir in candidates {
        if !dir.is_dir() {
            continue;
        }
        for ent in fs::read_dir(&dir).with_context(|| format!("读取目录失败: {}", dir.display()))? {
            let ent = ent.with_context(|| format!("读取目录项失败: {}", dir.display()))?;
            let ty = ent.file_type().context("读取文件类型失败")?;
            if !ty.is_file() {
                continue;
            }
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if !name.contains(lib_basename) {
                continue;
            }
            if name.ends_with(".dll.lib") || name.ends_with(".dll.a") {
                found.entry(name.to_string()).or_insert_with(|| ent.path());
            }
        }
    }
    Ok(found.into_values().collect())
}

fn generate_c_header_to_dist(
    workspace_root: &Path,
    dist_dir: &Path,
    pkg: &str,
    target: &str,
    profile: BuildProfile,
) -> anyhow::Result<()> {
    ensure_binary("cbindgen", "cbindgen")?;

    let crate_dir = workspace_root.join("crates").join(pkg);
    if !crate_dir.is_dir() {
        bail!("未找到 crate 目录: {}", crate_dir.display());
    }

    let include_dir = dist_dir
        .join(target)
        .join(match profile {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        })
        .join(pkg)
        .join("include");
    fs::create_dir_all(&include_dir).context("创建 include 目录失败")?;

    let header_path = include_dir.join(format!("{pkg}.h"));

    let mut cmd = Command::new("cbindgen");
    cmd.current_dir(workspace_root);
    cmd.arg("--lang").arg("c");
    cmd.arg("--crate").arg(pkg);
    cmd.arg("--output").arg(&header_path);
    cmd.arg(crate_dir);

    run_checked("cbindgen", &mut cmd)?;
    println!("dist: {}", header_path.display());
    Ok(())
}

fn find_artifact_path(
    out_dir: &Path,
    lib_basename: &str,
    target: &str,
    kind: ArtifactKind,
) -> anyhow::Result<PathBuf> {
    let is_windows = target.contains("windows");
    let is_macos = target.contains("apple-darwin");

    let path = match kind {
        ArtifactKind::Cdylib => {
            let file = if is_windows {
                format!("{lib_basename}.dll")
            } else if is_macos {
                format!("lib{lib_basename}.dylib")
            } else {
                format!("lib{lib_basename}.so")
            };
            out_dir.join(file)
        }
        ArtifactKind::Staticlib => {
            let file = if is_windows && target.contains("msvc") {
                format!("{lib_basename}.lib")
            } else {
                format!("lib{lib_basename}.a")
            };
            out_dir.join(file)
        }
    };

    if path.exists() {
        Ok(path)
    } else {
        bail!("产物文件不存在: {}", path.display())
    }
}

fn run_checked(name: &str, cmd: &mut Command) -> anyhow::Result<()> {
    let status = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("运行失败: {name}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("命令失败: {name} exit={status}")
    }
}

fn ensure_cargo_subcommand(sub: &str) -> anyhow::Result<()> {
    let ok = Command::new("cargo")
        .arg(sub)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        return Ok(());
    }

    let status = Command::new("cargo")
        .arg("install")
        .arg(format!("cargo-{sub}"))
        .arg("--locked")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("安装 cargo 子命令失败")?;
    if status.success() {
        Ok(())
    } else {
        bail!("安装 cargo 子命令失败: cargo-{sub}")
    }
}

fn ensure_binary(bin: &str, install_crate: &str) -> anyhow::Result<()> {
    let ok = Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        return Ok(());
    }

    let status = Command::new("cargo")
        .arg("install")
        .arg(install_crate)
        .arg("--locked")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("安装工具失败: {install_crate}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("安装工具失败: {install_crate}")
    }
}

fn workspace_root() -> anyhow::Result<PathBuf> {
    let out = Command::new("cargo")
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version=1")
        .output()
        .context("执行 cargo metadata 失败")?;
    if !out.status.success() {
        bail!("cargo metadata 执行失败")
    }
    let v: CargoMetadata = serde_json::from_slice(&out.stdout).context("解析 cargo metadata 失败")?;
    Ok(PathBuf::from(v.workspace_root))
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    workspace_root: String,
}

fn host_target_triple() -> anyhow::Result<String> {
    let out = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("执行 rustc -vV 失败")?;
    if !out.status.success() {
        bail!("rustc -vV 执行失败")
    }
    let text = String::from_utf8(out.stdout).context("rustc 输出不是 UTF-8")?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("host: ") {
            return Ok(rest.trim().to_string());
        }
    }
    bail!("未能解析 host target")
}

fn ensure_rust_target(target: &str) -> anyhow::Result<()> {
    if is_rust_std_installed(target)? {
        return Ok(());
    }

    let has_rustup = Command::new("rustup")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !has_rustup {
        bail!("目标未安装且 rustup 不可用: {target}");
    }

    let status = Command::new("rustup")
        .arg("target")
        .arg("add")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("执行 rustup target add 失败: {target}"))?;
    if !status.success() {
        bail!("rustup target add 失败: {target}")
    }
    if is_rust_std_installed(target)? {
        Ok(())
    } else {
        bail!("目标安装后仍缺少 std: {target}")
    }
}

fn is_rust_std_installed(target: &str) -> anyhow::Result<bool> {
    let out = Command::new("rustc")
        .arg("--print")
        .arg("target-libdir")
        .arg("--target")
        .arg(target)
        .stdin(Stdio::null())
        .output()
        .context("执行 rustc --print target-libdir 失败")?;
    if !out.status.success() {
        return Ok(false);
    }
    let s = String::from_utf8(out.stdout).context("rustc 输出不是 UTF-8")?;
    let dir = PathBuf::from(s.trim());
    if !dir.is_dir() {
        return Ok(false);
    }

    for ent in fs::read_dir(&dir).with_context(|| format!("读取目录失败: {}", dir.display()))? {
        let ent = ent.with_context(|| format!("读取目录项失败: {}", dir.display()))?;
        let ty = ent.file_type().context("读取文件类型失败")?;
        if !ty.is_file() {
            continue;
        }
        let name = ent.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("libstd-") && name.ends_with(".rlib") {
            return Ok(true);
        }
        if name.starts_with("libstd-") && name.ends_with(".a") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn common_targets() -> Vec<String> {
    vec![
        "x86_64-pc-windows-gnu".to_string(),
        "x86_64-pc-windows-msvc".to_string(),
        "x86_64-pc-windows-gnullvm".to_string(),
        "aarch64-pc-windows-msvc".to_string(),
        "aarch64-pc-windows-gnullvm".to_string(),
        "x86_64-unknown-linux-gnu".to_string(),
        "aarch64-unknown-linux-gnu".to_string(),
        "x86_64-unknown-linux-musl".to_string(),
        "aarch64-unknown-linux-musl".to_string(),
        "x86_64-apple-darwin".to_string(),
        "aarch64-apple-darwin".to_string(),
        "aarch64-linux-android".to_string(),
        "x86_64-linux-android".to_string(),
        "aarch64-apple-ios".to_string(),
        "aarch64-apple-ios-sim".to_string(),
    ]
}

fn map_windows_msvc_target_for_zigbuild(target: &str) -> Option<&'static str> {
    match target {
        "x86_64-pc-windows-msvc" => Some("x86_64-pc-windows-gnu"),
        "aarch64-pc-windows-msvc" => Some("aarch64-pc-windows-gnullvm"),
        _ => None,
    }
}

fn ensure_zig(version: &str) -> anyhow::Result<PathBuf> {
    let base = BaseDirs::new().ok_or_else(|| anyhow!("无法定位用户目录"))?;
    let cache_root = base.cache_dir().join("forgeffi").join("zig");
    let legacy_cache_root = base.cache_dir().join("tool-rs").join("zig");
    fs::create_dir_all(&cache_root).context("创建 Zig 缓存目录失败")?;

    let platform = ZigPlatform::detect()?;
    let install_dir = cache_root.join(version).join(platform.cache_key());
    fs::create_dir_all(&install_dir).context("创建 Zig 安装目录失败")?;

    let zig_path = platform.zig_bin_path(&install_dir);
    if zig_path.exists() {
        return Ok(zig_path);
    }

    let legacy_install_dir = legacy_cache_root.join(version).join(platform.cache_key());
    let legacy_zig_path = platform.zig_bin_path(&legacy_install_dir);
    if legacy_zig_path.exists() {
        copy_dir_all(&legacy_install_dir, &install_dir).context("复制旧 Zig 缓存目录失败")?;
        let zig_path = platform.zig_bin_path(&install_dir);
        if zig_path.exists() {
            return Ok(zig_path);
        }
    }

    let release = ZigRelease::for_platform(version, platform)?;
    let tmp = tempfile::tempdir().context("创建临时目录失败")?;
    let archive_path = tmp.path().join(release.archive_file_name());

    download_to_file(&release.url, &archive_path)?;
    verify_sha256(&archive_path, &release.sha256)?;
    extract_archive(&archive_path, tmp.path(), &release.archive_kind)?;

    let extracted_root = find_single_dir(tmp.path())
        .context("解压后未找到 Zig 根目录")?;
    copy_dir_all(&extracted_root, &install_dir).context("复制 Zig 目录失败")?;

    let zig_path = platform.zig_bin_path(&install_dir);
    if !zig_path.exists() {
        bail!("Zig 安装后仍未找到可执行文件: {}", zig_path.display());
    }
    Ok(zig_path)
}

#[derive(Copy, Clone, Debug)]
enum ArchiveKind {
    Zip,
    TarXz,
}

#[derive(Clone, Debug)]
struct ZigRelease {
    url: String,
    sha256: String,
    archive_kind: ArchiveKind,
}

impl ZigRelease {
    fn archive_file_name(&self) -> &'static str {
        match self.archive_kind {
            ArchiveKind::Zip => "zig.zip",
            ArchiveKind::TarXz => "zig.tar.xz",
        }
    }

    fn for_platform(version: &str, platform: ZigPlatform) -> anyhow::Result<ZigRelease> {
        let index_url = std::env::var("FORGEFFI_ZIG_INDEX_URL")
            .or_else(|_| std::env::var("TOOL_RS_ZIG_INDEX_URL"))
            .unwrap_or_else(|_| "https://ziglang.org/download/index.json".to_string());
        let index_text = ureq::get(&index_url)
            .call()
            .with_context(|| format!("下载 Zig index 失败: {index_url}"))?
            .into_string()
            .context("读取 Zig index 内容失败")?;

        let index: serde_json::Value =
            serde_json::from_str(&index_text).context("解析 Zig index.json 失败")?;
        let ver = index
            .get(version)
            .ok_or_else(|| anyhow!("Zig index 未包含该版本: {version}"))?;

        let key = platform.index_key();
        let plat = ver
            .get(key)
            .ok_or_else(|| anyhow!("Zig index 未包含该平台: {version} {key}"))?;
        let tarball = plat
            .get("tarball")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Zig index 缺少 tarball: {version} {key}"))?;
        let shasum = plat
            .get("shasum")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Zig index 缺少 shasum: {version} {key}"))?;

        let kind = if tarball.ends_with(".zip") {
            ArchiveKind::Zip
        } else if tarball.ends_with(".tar.xz") {
            ArchiveKind::TarXz
        } else {
            bail!("不支持的 Zig 压缩格式: {tarball}")
        };

        Ok(ZigRelease {
            url: tarball.to_string(),
            sha256: shasum.to_string(),
            archive_kind: kind,
        })
    }
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug)]
enum ZigPlatform {
    WindowsX86_64,
    LinuxX86_64,
    LinuxAarch64,
    MacosX86_64,
    MacosAarch64,
}

impl ZigPlatform {
    fn detect() -> anyhow::Result<ZigPlatform> {
        #[cfg(target_os = "windows")]
        {
            #[cfg(target_arch = "x86_64")]
            return Ok(ZigPlatform::WindowsX86_64);
            #[cfg(not(target_arch = "x86_64"))]
            bail!("当前 Windows 架构暂不支持自动下载 Zig")
        }
        #[cfg(target_os = "linux")]
        {
            #[cfg(target_arch = "x86_64")]
            return Ok(ZigPlatform::LinuxX86_64);
            #[cfg(target_arch = "aarch64")]
            return Ok(ZigPlatform::LinuxAarch64);
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            bail!("当前 Linux 架构暂不支持自动下载 Zig")
        }
        #[cfg(target_os = "macos")]
        {
            #[cfg(target_arch = "x86_64")]
            return Ok(ZigPlatform::MacosX86_64);
            #[cfg(target_arch = "aarch64")]
            return Ok(ZigPlatform::MacosAarch64);
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            bail!("当前 macOS 架构暂不支持自动下载 Zig")
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        {
            bail!("当前操作系统暂不支持自动下载 Zig")
        }
    }

    fn cache_key(self) -> &'static str {
        match self {
            ZigPlatform::WindowsX86_64 => "windows-x86_64",
            ZigPlatform::LinuxX86_64 => "linux-x86_64",
            ZigPlatform::LinuxAarch64 => "linux-aarch64",
            ZigPlatform::MacosX86_64 => "macos-x86_64",
            ZigPlatform::MacosAarch64 => "macos-aarch64",
        }
    }

    fn index_key(self) -> &'static str {
        match self {
            ZigPlatform::WindowsX86_64 => "x86_64-windows",
            ZigPlatform::LinuxX86_64 => "x86_64-linux",
            ZigPlatform::LinuxAarch64 => "aarch64-linux",
            ZigPlatform::MacosX86_64 => "x86_64-macos",
            ZigPlatform::MacosAarch64 => "aarch64-macos",
        }
    }

    fn zig_bin_path(self, install_dir: &Path) -> PathBuf {
        match self {
            ZigPlatform::WindowsX86_64 => install_dir.join("zig.exe"),
            _ => install_dir.join("zig"),
        }
    }
}

fn download_to_file(url: &str, out: &Path) -> anyhow::Result<()> {
    let resp = ureq::get(url)
        .call()
        .with_context(|| format!("下载 Zig 失败: {url}"))?;
    let mut reader = resp.into_reader();
    let mut file = fs::File::create(out).context("创建下载文件失败")?;
    std::io::copy(&mut reader, &mut file).context("写入下载文件失败")?;
    Ok(())
}

fn verify_sha256(path: &Path, expected_hex: &str) -> anyhow::Result<()> {
    let mut file = fs::File::open(path).context("打开下载文件失败")?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 1024 * 64];
    loop {
        let n = file.read(&mut buf).context("读取下载文件失败")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        bail!("SHA256 校验失败: expected={expected_hex} actual={actual}")
    }
}

fn extract_archive(archive: &Path, out_dir: &Path, kind: &ArchiveKind) -> anyhow::Result<()> {
    match kind {
        ArchiveKind::Zip => extract_zip(archive, out_dir),
        ArchiveKind::TarXz => extract_tar_xz(archive, out_dir),
    }
}

fn extract_zip(archive: &Path, out_dir: &Path) -> anyhow::Result<()> {
    let file = fs::File::open(archive).context("打开 zip 失败")?;
    let mut zip = zip::ZipArchive::new(file).context("解析 zip 失败")?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).context("读取 zip 条目失败")?;
        let outpath = match f.enclosed_name() {
            Some(p) => out_dir.join(p),
            None => continue,
        };
        if f.is_dir() {
            fs::create_dir_all(&outpath).context("创建目录失败")?;
            continue;
        }
        if let Some(parent) = outpath.parent() {
            fs::create_dir_all(parent).context("创建父目录失败")?;
        }
        let mut outfile = fs::File::create(&outpath).context("创建输出文件失败")?;
        std::io::copy(&mut f, &mut outfile).context("写入输出文件失败")?;
    }
    Ok(())
}

fn extract_tar_xz(archive: &Path, out_dir: &Path) -> anyhow::Result<()> {
    let file = fs::File::open(archive).context("打开 tar.xz 失败")?;
    let dec = xz2::read::XzDecoder::new(file);
    let mut ar = tar::Archive::new(dec);
    ar.unpack(out_dir).context("解压 tar.xz 失败")?;
    Ok(())
}

fn find_single_dir(root: &Path) -> anyhow::Result<PathBuf> {
    let mut dirs = Vec::new();
    for ent in fs::read_dir(root).context("读取目录失败")? {
        let ent = ent.context("读取目录项失败")?;
        let p = ent.path();
        if p.is_dir() {
            let name = p
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default();
            if name != "." && name != ".." {
                dirs.push(p);
            }
        }
    }
    if dirs.len() == 1 {
        Ok(dirs.remove(0))
    } else {
        bail!("解压根目录下目录数量异常: {}", dirs.len())
    }
}

fn copy_dir_all(src: &Path, dst: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dst).context("创建目标目录失败")?;
    for ent in fs::read_dir(src).context("读取源目录失败")? {
        let ent = ent.context("读取源目录项失败")?;
        let ty = ent.file_type().context("读取文件类型失败")?;
        let from = ent.path();
        let to = dst.join(ent.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else if ty.is_file() {
            fs::copy(&from, &to).with_context(|| {
                format!("复制文件失败: {} -> {}", from.display(), to.display())
            })?;
        }
    }
    Ok(())
}
