//! `anima`：引擎命令行工具。
//!
//! 它**不是**另一套实现，而是引擎的又一个宿主：所有操作都通过
//! [`am_core::Session::dispatch_json`] 走同一套方法协议，因此命令行能做的事
//! 编辑器也能做，反之亦然（这也是协议回归测试的一部分）。
//!
//! ```text
//! anima version
//! anima new   <dir> [--name demo] [--width 1024] [--height 1024]
//! anima info  <project>
//! anima validate <project>
//! anima render <project> -o out.png [--width 512] [--height 512] [--zoom 1]
//!                        [--time 1.5] [--motion idle] [--param AngleX=20]
//! anima export <project> [out.amproj]
//! anima import <file.amproj> <dir>
//! anima call   <project> <method> [params-json]
//! ```
//!
//! 退出码：0 成功；1 运行错误；2 用法错误。

use am_core::Session;
use am_format::{CreateOptions, Project};
use serde_json::{json, Value};
use std::process::ExitCode;

const USAGE: &str = "\
anima —— Anima 引擎命令行

用法:
  anima version
  anima new      <dir> [--name <name>] [--width <px>] [--height <px>]
  anima info     <project>
  anima validate <project>
  anima render   <project> -o <out.png> [--width <px>] [--height <px>]
                 [--zoom <z>] [--time <s>] [--motion <id>] [--param <id>=<v>]
  anima export   <project> [<out.amproj>]
  anima import   <file.amproj> <dir>
  anima call     <project> <method> [<params-json>]

<project> 可以是工程目录，也可以是 .amproj 压缩包。
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            eprintln!("用法错误：{message}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(Failure::Run(message)) => {
            eprintln!("错误：{message}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug)]
enum Failure {
    Usage(String),
    Run(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Usage(message) => write!(f, "用法错误：{message}"),
            Failure::Run(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Failure {}

impl From<String> for Failure {
    fn from(value: String) -> Self {
        Failure::Run(value)
    }
}

type Result<T> = std::result::Result<T, Failure>;

fn run(args: &[String]) -> Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(Failure::Usage("缺少子命令".into()));
    };
    let rest = &args[1..];
    match command {
        "version" | "--version" | "-v" => {
            let version = am_core::ENGINE_VERSION;
            println!(
                "anima {version}  (format {} v{}, {} 个方法)",
                am_format::FORMAT_ID,
                am_format::SDK_VERSION,
                am_core::METHODS.len()
            );
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(())
        }
        "new" => cmd_new(rest),
        "info" => cmd_info(rest),
        "validate" => cmd_validate(rest),
        "render" => cmd_render(rest),
        "export" => cmd_export(rest),
        "import" => cmd_import(rest),
        "call" => cmd_call(rest),
        other => Err(Failure::Usage(format!("未知子命令：{other}"))),
    }
}

// ---------------------------------------------------------------- 子命令

fn cmd_new(args: &[String]) -> Result<()> {
    let dir = args.first().ok_or_else(|| Failure::Usage("缺少目标目录".into()))?;
    let name = flag(args, "--name").unwrap_or_else(|| "untitled".to_string());
    let width = flag(args, "--width").and_then(|v| v.parse::<f32>().ok()).unwrap_or(1024.0);
    let height = flag(args, "--height").and_then(|v| v.parse::<f32>().ok()).unwrap_or(1024.0);

    let mut session = Session::empty();
    call(&mut session, "project.new", json!({ "name": name, "width": width, "height": height }))?;
    let spec = call(&mut session, "project.spec", json!({}))?;
    let spec: am_model::Spec =
        serde_json::from_value(spec).map_err(|e| Failure::Run(format!("描述层序列化失败：{e}")))?;

    let project = Project::create(&name, dir, CreateOptions::default())
        .map_err(|e| Failure::Run(e.to_string()))?;
    am_core::write_spec(&project, &spec).map_err(|e| Failure::Run(e.to_string()))?;
    println!("已创建工程：{}（{}）", dir, name);
    Ok(())
}

fn cmd_info(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| Failure::Usage("缺少工程路径".into()))?;
    let mut session = open(path)?;
    let stats = call(&mut session, "diagnostics.stats", json!({}))?;
    let model = call(&mut session, "doc.model", json!({}))?;
    println!("工程：{path}");
    println!("  模型：{}", model["name"].as_str().unwrap_or("?"));
    println!(
        "  画布：{} x {}",
        model["canvas"]["width"], model["canvas"]["height"]
    );
    for key in ["nodes", "parameters", "textures", "motions", "expressions", "physics"] {
        println!("  {key}：{}", stats[key]);
    }
    Ok(())
}

fn cmd_validate(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| Failure::Usage("缺少工程路径".into()))?;
    let project = Project::open(path).map_err(|e| Failure::Run(e.to_string()))?;
    let format_report = project.validate();
    let spec = am_core::read_spec(&project).map_err(|e| Failure::Run(e.to_string()))?;
    let model_report = spec.model.validate();

    if format_report.ok() && model_report.ok() {
        println!("✓ {path} 通过校验");
        return Ok(());
    }
    for issue in &format_report.issues {
        eprintln!("  [{}] {}", issue.code, issue.message);
    }
    for issue in &model_report.issues {
        eprintln!("  [{}] {}", issue.code, issue.message);
    }
    Err(Failure::Run(format!(
        "校验未通过：格式 {} 项、模型 {} 项",
        format_report.issues.len(),
        model_report.issues.len()
    )))
}

fn cmd_render(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| Failure::Usage("缺少工程路径".into()))?;
    let out = flag(args, "-o")
        .or_else(|| flag(args, "--out"))
        .ok_or_else(|| Failure::Usage("缺少输出文件 -o".into()))?;
    let width = flag(args, "--width").and_then(|v| v.parse::<u32>().ok()).unwrap_or(512);
    let height = flag(args, "--height").and_then(|v| v.parse::<u32>().ok()).unwrap_or(512);
    let zoom = flag(args, "--zoom").and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    let time = flag(args, "--time").and_then(|v| v.parse::<f32>().ok());
    let motion = flag(args, "--motion");
    let params = flags(args, "--param");

    let mut session = open(path)?;
    let init = call(&mut session, "renderer.init", json!({ "width": width, "height": height }));
    if let Err(message) = init {
        return Err(Failure::Run(format!("无法初始化渲染器：{message}")));
    }
    call(&mut session, "renderer.set_view", json!({ "zoom": zoom }))?;

    for entry in &params {
        let (id, value) = entry
            .split_once('=')
            .ok_or_else(|| Failure::Usage(format!("参数格式应为 id=value：{entry}")))?;
        let value: f64 = value
            .parse()
            .map_err(|_| Failure::Usage(format!("参数值不是数字：{entry}")))?;
        call(&mut session, "runtime.set_param", json!({ "id": id, "value": value }))?;
    }
    if let Some(id) = motion {
        call(&mut session, "motion.play", json!({ "id": id, "looping": false }))?;
    }
    if let Some(time) = time {
        // 用固定步长推进到指定时间，保证物理与动作有确定的历史
        let step = 1.0 / 60.0;
        let mut elapsed = 0.0f32;
        while elapsed < time {
            session.advance(step);
            elapsed += step;
        }
    }

    call(&mut session, "renderer.render", json!({}))?;
    call(&mut session, "renderer.save_png", json!({ "path": out }))?;
    println!("已渲染 {width}x{height} → {out}");
    Ok(())
}

fn cmd_export(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| Failure::Usage("缺少工程路径".into()))?;
    let out = args.get(1).map(String::as_str);
    let project = Project::open(path).map_err(|e| Failure::Run(e.to_string()))?;
    let result = project
        .export(out.map(std::path::Path::new), false)
        .map_err(|e| Failure::Run(e.to_string()))?;
    println!(
        "已导出 {}（{} 项，{} 字节）\n  sha256: {}",
        result.path, result.entries, result.bytes, result.sha256
    );
    Ok(())
}

fn cmd_import(args: &[String]) -> Result<()> {
    let source = args.first().ok_or_else(|| Failure::Usage("缺少 .amproj 文件".into()))?;
    let dest = args.get(1).ok_or_else(|| Failure::Usage("缺少目标目录".into()))?;
    let project = Project::import_from(source, dest).map_err(|e| Failure::Run(e.to_string()))?;
    println!("已导入到 {}（{} 个资源）", project.root().display(), project.list_assets().map(|a| a.len()).unwrap_or(0));
    Ok(())
}

fn cmd_call(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| Failure::Usage("缺少工程路径".into()))?;
    let method = args.get(1).ok_or_else(|| Failure::Usage("缺少方法名".into()))?;
    let params = args.get(2).map(String::as_str).unwrap_or("{}");
    let mut session = open(path)?;
    let raw = session.dispatch_json(method, params);
    println!("{raw}");
    if raw.starts_with(r#"{"ok":false"#) {
        return Err(Failure::Run(format!("{method} 调用失败")));
    }
    Ok(())
}

// ---------------------------------------------------------------- 辅助

/// 打开工程并返回会话。
fn open(path: &str) -> Result<Session> {
    let project = Project::open(path).map_err(|e| Failure::Run(e.to_string()))?;
    let spec = am_core::read_spec(&project).map_err(|e| Failure::Run(e.to_string()))?;
    Ok(Session::with_spec(spec))
}

/// 调用引擎方法并返回 `result`。
fn call(session: &mut Session, method: &str, params: Value) -> Result<Value> {
    let raw = session.dispatch_json(method, &params.to_string());
    let envelope: Value = serde_json::from_str(&raw)
        .map_err(|e| Failure::Run(format!("{method} 返回了非法 JSON：{e}")))?;
    if envelope["ok"] == Value::Bool(false) {
        return Err(Failure::Run(format!(
            "{method} 失败：{}",
            envelope["error"]["message"].as_str().unwrap_or("未知错误")
        )));
    }
    Ok(envelope["result"].clone())
}

/// 取 `--flag value` 形式的参数（最后一个生效）。
fn flag(args: &[String], name: &str) -> Option<String> {
    let mut result = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == name {
            if let Some(value) = iter.next() {
                result = Some(value.clone());
            }
        }
    }
    result
}

/// 取可重复的 `--flag value` 参数。
fn flags(args: &[String], name: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == name {
            if let Some(value) = iter.next() {
                result.push(value.clone());
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_are_parsed() {
        let list = args(&["a", "--width", "64", "--width", "128", "-o", "out.png"]);
        assert_eq!(flag(&list, "--width").as_deref(), Some("128"));
        assert_eq!(flag(&list, "-o").as_deref(), Some("out.png"));
        assert_eq!(flag(&list, "--missing"), None);
        assert_eq!(flags(&list, "--width"), vec!["64", "128"]);
    }

    #[test]
    fn version_command_succeeds() {
        assert!(run(&args(&["version"])).is_ok());
    }

    #[test]
    fn unknown_command_is_usage_error() {
        assert!(matches!(run(&args(&["nope"])), Err(Failure::Usage(_))));
        assert!(matches!(run(&[]), Err(Failure::Usage(_))));
    }

    #[test]
    fn missing_path_is_usage_error() {
        assert!(matches!(run(&args(&["info"])), Err(Failure::Usage(_))));
        assert!(matches!(run(&args(&["render", "x"])), Err(Failure::Usage(_))));
    }

    #[test]
    fn call_reports_engine_errors() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("demo");
        let path = root.to_string_lossy().to_string();
        cmd_new(&vec![path.clone(), "--name".into(), "demo".into()]).unwrap();

        let mut session = open(&path).unwrap();
        assert!(call(&mut session, "system.ping", json!({})).is_ok());
        assert!(call(&mut session, "nope.nope", json!({})).is_err());
    }

    #[test]
    fn new_then_info_works() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("demo");
        let path = root.to_string_lossy().to_string();
        cmd_new(&vec![
            path.clone(),
            "--name".into(),
            "demo".into(),
            "--width".into(),
            "320".into(),
            "--height".into(),
            "240".into(),
        ])
        .unwrap();
        assert!(root.join("spec").join("model.json").is_file());
        assert!(cmd_info(&vec![path.clone()]).is_ok());
        assert!(cmd_validate(&vec![path]).is_ok());
    }

    #[test]
    fn call_command_reports_failures() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("demo");
        let path = root.to_string_lossy().to_string();
        cmd_new(&vec![path.clone(), "--name".into(), "demo".into()]).unwrap();
        assert!(cmd_call(&vec![path.clone(), "system.ping".into()]).is_ok());
        assert!(cmd_call(&vec![path, "nope.nope".into()]).is_err());
    }
}
