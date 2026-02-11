#![allow(non_snake_case)]

use crate::app_config::AppType;
use crate::init_status::{InitErrorPayload, SkillsMigrationPayload};
use crate::services::ProviderService;
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::Path;
use std::str::FromStr;
use tauri::AppHandle;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// 打开外部链接
#[tauri::command]
pub async fn open_external(app: AppHandle, url: String) -> Result<bool, String> {
    let url = if url.starts_with("http://") || url.starts_with("https://") {
        url
    } else {
        format!("https://{url}")
    };

    app.opener()
        .open_url(&url, None::<String>)
        .map_err(|e| format!("打开链接失败: {e}"))?;

    Ok(true)
}

/// 检查更新
#[tauri::command]
pub async fn check_for_updates(handle: AppHandle) -> Result<bool, String> {
    handle
        .opener()
        .open_url(
            "https://github.com/farion1231/cc-switch/releases/latest",
            None::<String>,
        )
        .map_err(|e| format!("打开更新页面失败: {e}"))?;

    Ok(true)
}

/// 判断是否为便携版（绿色版）运行
#[tauri::command]
pub async fn is_portable_mode() -> Result<bool, String> {
    let exe_path = std::env::current_exe().map_err(|e| format!("获取可执行路径失败: {e}"))?;
    if let Some(dir) = exe_path.parent() {
        Ok(dir.join("portable.ini").is_file())
    } else {
        Ok(false)
    }
}

/// 获取应用启动阶段的初始化错误（若有）。
/// 用于前端在早期主动拉取，避免事件订阅竞态导致的提示缺失。
#[tauri::command]
pub async fn get_init_error() -> Result<Option<InitErrorPayload>, String> {
    Ok(crate::init_status::get_init_error())
}

/// 获取 JSON→SQLite 迁移结果（若有）。
/// 只返回一次 true，之后返回 false，用于前端显示一次性 Toast 通知。
#[tauri::command]
pub async fn get_migration_result() -> Result<bool, String> {
    Ok(crate::init_status::take_migration_success())
}

/// 获取 Skills 自动导入（SSOT）迁移结果（若有）。
/// 只返回一次 Some({count})，之后返回 None，用于前端显示一次性 Toast 通知。
#[tauri::command]
pub async fn get_skills_migration_result() -> Result<Option<SkillsMigrationPayload>, String> {
    Ok(crate::init_status::take_skills_migration_result())
}

#[derive(serde::Serialize)]
pub struct ToolVersion {
    name: String,
    version: Option<String>,
    latest_version: Option<String>, // 新增字段：最新版本
    error: Option<String>,
}

#[tauri::command]
pub async fn get_tool_versions() -> Result<Vec<ToolVersion>, String> {
    let tools = vec!["nodejs", "claude", "codex", "gemini", "opencode"];
    let mut results = Vec::new();

    // 使用全局 HTTP 客户端（已包含代理配置）
    let client = crate::proxy::http_client::get();

    for tool in tools {
        // 1. 获取本地版本
        let (local_version, local_error) = if tool == "nodejs" {
            // 先尝试直接执行
            let direct_result = try_get_nodejs_version();
            if direct_result.0.is_some() {
                direct_result
            } else {
                // 扫描常见的 node 安装路径
                scan_nodejs_version()
            }
        } else if let Some(distro) = wsl_distro_for_tool(tool) {
            try_get_version_wsl(tool, &distro)
        } else {
            // 先尝试直接执行
            let direct_result = try_get_version(tool);

            if direct_result.0.is_some() {
                direct_result
            } else {
                // 扫描常见的 npm 全局安装路径
                scan_cli_version(tool)
            }
        };

        // 2. 获取远程最新版本
        let latest_version = match tool {
            "nodejs" => fetch_npm_latest_version(&client, "node").await,
            "claude" => fetch_npm_latest_version(&client, "@anthropic-ai/claude-code").await,
            "codex" => fetch_npm_latest_version(&client, "@openai/codex").await,
            "gemini" => fetch_npm_latest_version(&client, "@google/gemini-cli").await,
            "opencode" => fetch_npm_latest_version(&client, "opencode-ai").await,
            _ => None,
        };

        results.push(ToolVersion {
            name: tool.to_string(),
            version: local_version,
            latest_version,
            error: local_error,
        });
    }

    Ok(results)
}

/// Helper function to fetch latest version from npm registry
async fn fetch_npm_latest_version(client: &reqwest::Client, package: &str) -> Option<String> {
    let url = format!("https://registry.npmjs.org/{package}");
    match client.get(&url).send().await {
        Ok(resp) => {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                json.get("dist-tags")
                    .and_then(|tags| tags.get("latest"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

#[allow(dead_code)]
/// Helper function to fetch latest version from GitHub releases
async fn fetch_github_latest_version(client: &reqwest::Client, repo: &str) -> Option<String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    match client
        .get(&url)
        .header("User-Agent", "cc-switch")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(resp) => {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                json.get("tag_name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.strip_prefix('v').unwrap_or(s).to_string())
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

/// 预编译的版本号正则表达式
static VERSION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\d+\.\d+\.\d+(-[\w.]+)?").expect("Invalid version regex"));

/// 从版本输出中提取纯版本号
fn extract_version(raw: &str) -> String {
    VERSION_RE
        .find(raw)
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| raw.to_string())
}

/// 尝试直接执行命令获取版本
fn try_get_version(tool: &str) -> (Option<String>, Option<String>) {
    try_get_version_with_command(tool, &format!("{tool} --version"))
}

/// 获取 Node.js 版本（使用 --version 参数）
fn try_get_nodejs_version() -> (Option<String>, Option<String>) {
    try_get_version_with_command("node", "node --version")
}

/// 通用版本检测函数
fn try_get_version_with_command(_tool: &str, cmd: &str) -> (Option<String>, Option<String>) {
    use std::process::Command;

    #[cfg(target_os = "windows")]
    let output = {
        Command::new("cmd")
            .args(["/C", cmd])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
    };

    #[cfg(not(target_os = "windows"))]
    let output = { Command::new("sh").arg("-c").arg(cmd).output() };

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if out.status.success() {
                let raw = if stdout.is_empty() { &stderr } else { &stdout };
                if raw.is_empty() {
                    (None, Some("not installed or not executable".to_string()))
                } else {
                    (Some(extract_version(raw)), None)
                }
            } else {
                let err = if stderr.is_empty() { stdout } else { stderr };
                (
                    None,
                    Some(if err.is_empty() {
                        "not installed or not executable".to_string()
                    } else {
                        err
                    }),
                )
            }
        }
        Err(e) => (None, Some(e.to_string())),
    }
}

/// 校验 WSL 发行版名称是否合法
/// WSL 发行版名称只允许字母、数字、连字符和下划线
#[cfg(target_os = "windows")]
fn is_valid_wsl_distro_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

#[cfg(target_os = "windows")]
fn try_get_version_wsl(tool: &str, distro: &str) -> (Option<String>, Option<String>) {
    use std::process::Command;

    // 防御性断言：tool 只能是预定义的值
    debug_assert!(
        ["claude", "codex", "gemini", "opencode"].contains(&tool),
        "unexpected tool name: {tool}"
    );

    // 校验 distro 名称，防止命令注入
    if !is_valid_wsl_distro_name(distro) {
        return (None, Some(format!("[WSL:{distro}] invalid distro name")));
    }

    let output = Command::new("wsl.exe")
        .args([
            "-d",
            distro,
            "--",
            "sh",
            "-lc",
            &format!("{tool} --version"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if out.status.success() {
                let raw = if stdout.is_empty() { &stderr } else { &stdout };
                if raw.is_empty() {
                    (
                        None,
                        Some(format!("[WSL:{distro}] not installed or not executable")),
                    )
                } else {
                    (Some(extract_version(raw)), None)
                }
            } else {
                let err = if stderr.is_empty() { stdout } else { stderr };
                (
                    None,
                    Some(format!(
                        "[WSL:{distro}] {}",
                        if err.is_empty() {
                            "not installed or not executable".to_string()
                        } else {
                            err
                        }
                    )),
                )
            }
        }
        Err(e) => (None, Some(format!("[WSL:{distro}] exec failed: {e}"))),
    }
}

/// 非 Windows 平台的 WSL 版本检测存根
/// 注意：此函数实际上不会被调用，因为 `wsl_distro_from_path` 在非 Windows 平台总是返回 None。
/// 保留此函数是为了保持 API 一致性，防止未来重构时遗漏。
#[cfg(not(target_os = "windows"))]
fn try_get_version_wsl(_tool: &str, _distro: &str) -> (Option<String>, Option<String>) {
    (
        None,
        Some("WSL check not supported on this platform".to_string()),
    )
}

/// 扫描常见路径查找 CLI
fn scan_cli_version(tool: &str) -> (Option<String>, Option<String>) {
    use std::process::Command;

    let home = dirs::home_dir().unwrap_or_default();

    // 常见的安装路径（原生安装优先）
    let mut search_paths: Vec<std::path::PathBuf> = vec![
        home.join(".local/bin"), // Native install (official recommended)
        home.join(".npm-global/bin"),
        home.join("n/bin"), // n version manager
    ];

    #[cfg(target_os = "macos")]
    {
        search_paths.push(std::path::PathBuf::from("/opt/homebrew/bin"));
        search_paths.push(std::path::PathBuf::from("/usr/local/bin"));
    }

    #[cfg(target_os = "linux")]
    {
        search_paths.push(std::path::PathBuf::from("/usr/local/bin"));
        search_paths.push(std::path::PathBuf::from("/usr/bin"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = dirs::data_dir() {
            search_paths.push(appdata.join("npm"));
        }
        search_paths.push(std::path::PathBuf::from("C:\\Program Files\\nodejs"));
    }

    // 添加 fnm 路径支持
    let fnm_base = home.join(".local/state/fnm_multishells");
    if fnm_base.exists() {
        if let Ok(entries) = std::fs::read_dir(&fnm_base) {
            for entry in entries.flatten() {
                let bin_path = entry.path().join("bin");
                if bin_path.exists() {
                    search_paths.push(bin_path);
                }
            }
        }
    }

    // 扫描 nvm 目录下的所有 node 版本
    let nvm_base = home.join(".nvm/versions/node");
    if nvm_base.exists() {
        if let Ok(entries) = std::fs::read_dir(&nvm_base) {
            for entry in entries.flatten() {
                let bin_path = entry.path().join("bin");
                if bin_path.exists() {
                    search_paths.push(bin_path);
                }
            }
        }
    }

    // 添加 Go 路径支持 (opencode 使用 go install 安装)
    if tool == "opencode" {
        search_paths.push(home.join("go/bin")); // go install 默认路径
        if let Ok(gopath) = std::env::var("GOPATH") {
            search_paths.push(std::path::PathBuf::from(gopath).join("bin"));
        }
    }

    // 在每个路径中查找工具
    for path in &search_paths {
        let tool_path = if cfg!(target_os = "windows") {
            path.join(format!("{tool}.cmd"))
        } else {
            path.join(tool)
        };

        if tool_path.exists() {
            // 构建 PATH 环境变量，确保 node 可被找到
            let current_path = std::env::var("PATH").unwrap_or_default();

            #[cfg(target_os = "windows")]
            let new_path = format!("{};{}", path.display(), current_path);

            #[cfg(not(target_os = "windows"))]
            let new_path = format!("{}:{}", path.display(), current_path);

            #[cfg(target_os = "windows")]
            let output = {
                // 使用 cmd /C 包装执行，确保子进程也在隐藏的控制台中运行
                Command::new("cmd")
                    .args(["/C", &format!("\"{}\" --version", tool_path.display())])
                    .env("PATH", &new_path)
                    .creation_flags(CREATE_NO_WINDOW)
                    .output()
            };

            #[cfg(not(target_os = "windows"))]
            let output = {
                Command::new(&tool_path)
                    .arg("--version")
                    .env("PATH", &new_path)
                    .output()
            };

            if let Ok(out) = output {
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                if out.status.success() {
                    let raw = if stdout.is_empty() { &stderr } else { &stdout };
                    if !raw.is_empty() {
                        return (Some(extract_version(raw)), None);
                    }
                }
            }
        }
    }

    (None, Some("not installed or not executable".to_string()))
}

/// 扫描常见路径查找 Node.js
/// 用于解决 macOS GUI 应用 PATH 环境变量不包含用户安装的 node 路径的问题
fn scan_nodejs_version() -> (Option<String>, Option<String>) {
    use std::process::Command;

    let home = dirs::home_dir().unwrap_or_default();

    // 常见的 node 安装路径
    let mut search_paths: Vec<std::path::PathBuf> = vec![
        home.join(".local/bin"),
        home.join(".npm-global/bin"),
        home.join("n/bin"), // n version manager
    ];

    #[cfg(target_os = "macos")]
    {
        search_paths.push(std::path::PathBuf::from("/opt/homebrew/bin")); // Apple Silicon Homebrew
        search_paths.push(std::path::PathBuf::from("/usr/local/bin")); // Intel Homebrew
    }

    #[cfg(target_os = "linux")]
    {
        search_paths.push(std::path::PathBuf::from("/usr/local/bin"));
        search_paths.push(std::path::PathBuf::from("/usr/bin"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = dirs::data_dir() {
            search_paths.push(appdata.join("npm"));
        }
        search_paths.push(std::path::PathBuf::from("C:\\Program Files\\nodejs"));
    }

    // 添加 fnm 路径支持
    let fnm_base = home.join(".local/state/fnm_multishells");
    if fnm_base.exists() {
        if let Ok(entries) = std::fs::read_dir(&fnm_base) {
            for entry in entries.flatten() {
                let bin_path = entry.path().join("bin");
                if bin_path.exists() {
                    search_paths.push(bin_path);
                }
            }
        }
    }

    // 扫描 nvm 目录下的所有 node 版本
    let nvm_base = home.join(".nvm/versions/node");
    if nvm_base.exists() {
        if let Ok(entries) = std::fs::read_dir(&nvm_base) {
            for entry in entries.flatten() {
                let bin_path = entry.path().join("bin");
                if bin_path.exists() {
                    search_paths.push(bin_path);
                }
            }
        }
    }

    // 在每个路径中查找 node
    for path in &search_paths {
        let node_path = if cfg!(target_os = "windows") {
            path.join("node.exe")
        } else {
            path.join("node")
        };

        if node_path.exists() {
            // 构建 PATH 环境变量
            let current_path = std::env::var("PATH").unwrap_or_default();

            #[cfg(target_os = "windows")]
            let new_path = format!("{};{}", path.display(), current_path);

            #[cfg(not(target_os = "windows"))]
            let new_path = format!("{}:{}", path.display(), current_path);

            #[cfg(target_os = "windows")]
            let output = {
                Command::new("cmd")
                    .args(["/C", &format!("\"{}\" --version", node_path.display())])
                    .env("PATH", &new_path)
                    .creation_flags(CREATE_NO_WINDOW)
                    .output()
            };

            #[cfg(not(target_os = "windows"))]
            let output = {
                Command::new(&node_path)
                    .arg("--version")
                    .env("PATH", &new_path)
                    .output()
            };

            if let Ok(out) = output {
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                if out.status.success() {
                    let raw = if stdout.is_empty() { &stderr } else { &stdout };
                    if !raw.is_empty() {
                        return (Some(extract_version(raw)), None);
                    }
                }
            }
        }
    }

    (None, Some("not installed or not executable".to_string()))
}

fn wsl_distro_for_tool(tool: &str) -> Option<String> {
    let override_dir = match tool {
        "claude" => crate::settings::get_claude_override_dir(),
        "codex" => crate::settings::get_codex_override_dir(),
        "gemini" => crate::settings::get_gemini_override_dir(),
        "opencode" => crate::settings::get_opencode_override_dir(),
        _ => None,
    }?;

    wsl_distro_from_path(&override_dir)
}

/// 从 UNC 路径中提取 WSL 发行版名称
/// 支持 `\\wsl$\Ubuntu\...` 和 `\\wsl.localhost\Ubuntu\...` 两种格式
#[cfg(target_os = "windows")]
fn wsl_distro_from_path(path: &Path) -> Option<String> {
    use std::path::{Component, Prefix};
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return None;
    };
    match prefix.kind() {
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            let server_name = server.to_string_lossy();
            if server_name.eq_ignore_ascii_case("wsl$")
                || server_name.eq_ignore_ascii_case("wsl.localhost")
            {
                let distro = share.to_string_lossy().to_string();
                if !distro.is_empty() {
                    return Some(distro);
                }
            }
            None
        }
        _ => None,
    }
}

/// 非 Windows 平台不支持 WSL 路径解析
#[cfg(not(target_os = "windows"))]
fn wsl_distro_from_path(_path: &Path) -> Option<String> {
    None
}

// ============================================================
// CLI 工具安装/升级
// ============================================================

/// CLI 工具安装/升级操作类型
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase")]
pub enum CliToolAction {
    Install,
    Upgrade,
}

/// CLI 工具安装/升级结果
#[derive(serde::Serialize)]
pub struct CliToolInstallResult {
    success: bool,
    tool: String,
    action: CliToolAction,
    message: String,
    output: String,
    error: Option<String>,
}

/// 获取工具对应的 npm 包名
fn get_npm_package_for_tool(tool: &str) -> Option<&'static str> {
    match tool {
        "claude" => Some("@anthropic-ai/claude-code"),
        "codex" => Some("@openai/codex"),
        "gemini" => Some("@google/gemini-cli"),
        "opencode" => Some("opencode-ai"),
        _ => None,
    }
}

/// 安装或升级 CLI 工具
#[tauri::command]
pub async fn install_cli_tool(
    tool: String,
    action: CliToolAction,
) -> Result<CliToolInstallResult, String> {
    log::info!("[CLI安装] 收到请求: tool={}, action={:?}", tool, action);

    // 验证工具名称
    if !["claude", "codex", "gemini", "opencode"].contains(&tool.as_str()) {
        log::error!("[CLI安装] 不支持的工具: {}", tool);
        return Ok(CliToolInstallResult {
            success: false,
            tool: tool.clone(),
            action: action.clone(),
            message: format!("不支持的工具: {tool}"),
            output: String::new(),
            error: Some("Unsupported tool".to_string()),
        });
    }

    let package =
        get_npm_package_for_tool(&tool).ok_or_else(|| format!("工具 {tool} 不支持 npm 安装"))?;
    log::info!("[CLI安装] npm包名: {}", package);

    // 跨平台执行命令
    let output = {
        #[cfg(target_os = "windows")]
        {
            // Windows 上使用 npm.cmd，添加 --force 绕过缓存问题
            let args = if matches!(action, CliToolAction::Upgrade) {
                vec![package.to_string() + "@latest"]
            } else {
                vec![package.to_string()]
            };
            log::info!("[CLI安装] 执行命令: npm.cmd install -g --force {}", args[0]);
            std::process::Command::new("npm.cmd")
                .arg("install")
                .arg("-g")
                .arg("--force")
                .args(&args)
                .creation_flags(CREATE_NO_WINDOW)
                .output()
        }

        #[cfg(not(target_os = "windows"))]
        {
            let npm_cmd = if matches!(action, CliToolAction::Upgrade) {
                format!(r#"npm install -g --force {}@latest"#, package)
            } else {
                format!(r#"npm install -g --force {}"#, package)
            };
            log::info!("[CLI安装] 执行命令: {}", npm_cmd);

            // 先尝试使用 sudo（如果已授权，5分钟内有效）
            let sudo_npm_cmd = format!(r#"cd /tmp && sudo -n {}"#, npm_cmd);
            let output_with_sudo = std::process::Command::new("sh")
                .arg("-c")
                .arg(&sudo_npm_cmd)
                .output()
                .map_err(|_| ());

            // 检查 sudo 是否需要密码
            let use_cached_auth = match &output_with_sudo {
                Ok(out) => out.status.success(),
                Err(_) => false,
            };

            if use_cached_auth {
                log::info!("[CLI安装] 使用缓存的 sudo 授权");
                output_with_sudo.map_err(|_| "执行 npm 命令失败".to_string())
            } else {
                // 尝试不使用 sudo
                let output_no_sudo = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&npm_cmd)
                    .output()
                    .map_err(|_| ());

                match output_no_sudo {
                    Ok(out) if out.status.success() => {
                        log::info!("[CLI安装] 无需授权即可完成");
                        Ok(out)
                    }
                    _ => {
                        // 权限不足，使用 macOS 授权，并延长 sudo 时间戳
                        log::info!("[CLI安装] 权限不足，使用系统授权");

                        // 先执行 sudo -v 延长授权时间（5分钟），然后再执行实际命令
                        let full_cmd = format!(r#"cd /tmp && sudo -v && {}"#, npm_cmd);

                        // 转义引号和反斜杠以便在 AppleScript 中使用
                        let escaped_cmd = full_cmd.replace('"', r#"\""#).replace('\\', r#"\\"#);

                        let apple_script = format!(
                            r#"do shell script "{}" with administrator privileges"#,
                            escaped_cmd
                        );

                        std::process::Command::new("osascript")
                            .arg("-e")
                            .arg(&apple_script)
                            .output()
                            .map_err(|_| "执行授权命令失败".to_string())
                    }
                }
            }
        }
    };

    let output = match output {
        Ok(out) => {
            log::info!(
                "[CLI安装] 命令执行完成, exit code: {}",
                out.status.code().unwrap_or(-1)
            );
            out
        }
        Err(e) => {
            log::error!("[CLI安装] 命令执行失败: {}", e);
            return Err(format!("执行 npm 命令失败: {e}"));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    log::info!("[CLI安装] stdout: {}", stdout);
    if !stderr.is_empty() {
        log::warn!("[CLI安装] stderr: {}", stderr);
    }

    if output.status.success() {
        // 验证安装结果
        let (version, _) = try_get_version(&tool);
        let version_msg = version
            .as_ref()
            .map(|v| format!("当前版本: {v}"))
            .unwrap_or_else(|| "版本检测失败，请手动验证".to_string());

        Ok(CliToolInstallResult {
            success: true,
            tool: tool.clone(),
            action: action.clone(),
            message: format!(
                "{}成功，{version_msg}",
                match action {
                    CliToolAction::Install => "安装",
                    CliToolAction::Upgrade => "升级",
                }
            ),
            output: stdout.clone(),
            error: None,
        })
    } else {
        let error_msg = if stderr.is_empty() {
            stdout.clone()
        } else {
            stderr
        };
        Ok(CliToolInstallResult {
            success: false,
            tool: tool.clone(),
            action: action.clone(),
            message: format!(
                "{}失败",
                match action {
                    CliToolAction::Install => "安装",
                    CliToolAction::Upgrade => "升级",
                }
            ),
            output: stdout,
            error: Some(error_msg),
        })
    }
}

/// 打开指定提供商的终端
///
/// 根据提供商配置的环境变量启动一个带有该提供商特定设置的终端
/// 无需检查是否为当前激活的提供商，任何提供商都可以打开终端
#[allow(non_snake_case)]
#[tauri::command]
pub async fn open_provider_terminal(
    state: State<'_, crate::store::AppState>,
    app: String,
    #[allow(non_snake_case)] providerId: String,
    #[allow(non_snake_case)] workingDirectory: Option<String>,
) -> Result<bool, String> {
    let app_type = AppType::from_str(&app).map_err(|e| e.to_string())?;

    // 获取提供商配置
    let providers = ProviderService::list(state.inner(), app_type.clone())
        .map_err(|e| format!("获取提供商列表失败: {e}"))?;

    let provider = providers
        .get(&providerId)
        .ok_or_else(|| format!("提供商 {providerId} 不存在"))?;

    // 从提供商配置中提取环境变量
    let config = &provider.settings_config;
    let env_vars = extract_env_vars_from_config(config, &app_type);

    // 解析工作目录路径
    let working_dir = workingDirectory
        .as_ref()
        .map(|p| std::path::Path::new(p))
        .filter(|p| p.is_absolute());

    // 根据平台启动终端，传入提供商ID用于生成唯一的配置文件名
    launch_terminal_with_env(env_vars, &providerId, working_dir, &app_type)
        .map_err(|e| format!("启动终端失败: {e}"))?;

    Ok(true)
}

/// 从提供商配置中提取环境变量
pub fn extract_env_vars_from_config(
    config: &serde_json::Value,
    app_type: &AppType,
) -> Vec<(String, String)> {
    let mut env_vars = Vec::new();

    let Some(obj) = config.as_object() else {
        return env_vars;
    };

    // 处理 env 字段（Claude/Gemini 通用）
    if let Some(env) = obj.get("env").and_then(|v| v.as_object()) {
        for (key, value) in env {
            if let Some(str_val) = value.as_str() {
                env_vars.push((key.clone(), str_val.to_string()));
            }
        }

        // 处理 base_url: 根据应用类型添加对应的环境变量
        let base_url_key = match app_type {
            AppType::Claude => Some("ANTHROPIC_BASE_URL"),
            AppType::Gemini => Some("GOOGLE_GEMINI_BASE_URL"),
            _ => None,
        };

        if let Some(key) = base_url_key {
            if let Some(url_str) = env.get(key).and_then(|v| v.as_str()) {
                env_vars.push((key.to_string(), url_str.to_string()));
            }
        }
    }

    // Codex 使用 auth 字段转换为 OPENAI_API_KEY
    if *app_type == AppType::Codex {
        if let Some(auth) = obj.get("auth").and_then(|v| v.as_str()) {
            env_vars.push(("OPENAI_API_KEY".to_string(), auth.to_string()));
        }
    }

    // Gemini 使用 api_key 字段转换为 GEMINI_API_KEY
    if *app_type == AppType::Gemini {
        if let Some(api_key) = obj.get("api_key").and_then(|v| v.as_str()) {
            env_vars.push(("GEMINI_API_KEY".to_string(), api_key.to_string()));
        }
    }

    env_vars
}

/// 获取 CLI 命令名称
fn get_cli_command(app_type: &AppType) -> &str {
    match app_type {
        AppType::Claude => "claude",
        AppType::Codex => "codex",
        AppType::Gemini => "gemini",
        AppType::OpenCode => "opencode",
    }
}

/// 创建临时配置文件并启动对应 CLI 的终端
/// 只有 Claude 需要 --settings 参数传入配置文件，其他 CLI 直接启动
pub fn launch_terminal_with_env(
    env_vars: Vec<(String, String)>,
    provider_id: &str,
    working_dir: Option<&std::path::Path>,
    app_type: &AppType,
) -> Result<(), String> {
    let temp_dir = std::env::temp_dir();
    let cli_command = get_cli_command(app_type);

    // 只有 Claude 需要配置文件，其他 CLI 直接通过环境变量启动
    let config_file = if *app_type == AppType::Claude {
        let file = temp_dir.join(format!(
            "claude_{}_{}.json",
            provider_id,
            std::process::id()
        ));
        // 创建并写入 Claude 配置文件
        write_claude_config(&file, &env_vars)?;
        Some(file)
    } else {
        None
    };

    #[cfg(target_os = "macos")]
    {
        launch_macos_terminal(cli_command, &config_file, working_dir, app_type)?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        launch_linux_terminal(cli_command, &config_file, working_dir, app_type)?;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        launch_windows_terminal(
            cli_command,
            &config_file,
            &temp_dir,
            provider_id,
            working_dir,
            app_type,
        )?;
        return Ok(());
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    Err("不支持的操作系统".to_string())
}

/// 写入 claude 配置文件
fn write_claude_config(
    config_file: &std::path::Path,
    env_vars: &[(String, String)],
) -> Result<(), String> {
    let mut config_obj = serde_json::Map::new();
    let mut env_obj = serde_json::Map::new();

    for (key, value) in env_vars {
        env_obj.insert(key.clone(), serde_json::Value::String(value.clone()));
    }

    config_obj.insert("env".to_string(), serde_json::Value::Object(env_obj));

    let config_json =
        serde_json::to_string_pretty(&config_obj).map_err(|e| format!("序列化配置失败: {e}"))?;

    std::fs::write(config_file, config_json).map_err(|e| format!("写入配置文件失败: {e}"))
}

/// macOS: 根据用户首选终端启动
#[cfg(target_os = "macos")]
fn launch_macos_terminal(
    cli_command: &str,
    config_file: &Option<std::path::PathBuf>,
    working_dir: Option<&std::path::Path>,
    app_type: &AppType,
) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let preferred = crate::settings::get_preferred_terminal();
    let terminal = preferred.as_deref().unwrap_or("terminal");

    let temp_dir = std::env::temp_dir();
    let script_file = temp_dir.join(format!("cc_switch_launcher_{}.sh", std::process::id()));

    // 构建工作目录的 cd 命令（如果提供）
    let cd_command = if let Some(dir) = working_dir {
        let dir_path = dir.to_string_lossy();
        format!(r#"cd "{dir_path}""#)
    } else {
        String::new()
    };

    // 根据应用类型构建启动命令
    let launch_command = if *app_type == AppType::Claude {
        // Claude 需要 --settings 参数
        let config_path = config_file.as_ref().unwrap().to_string_lossy();
        format!(r#"{cli_command} --settings "{config_path}""#)
    } else {
        // 其他 CLI 直接启动
        cli_command.to_string()
    };

    // 清理命令（Claude 需要清理配置文件）
    let cleanup_command = if *app_type == AppType::Claude {
        let config_path = config_file.as_ref().unwrap().to_string_lossy();
        format!(r#"rm -f "{config_path}""#)
    } else {
        String::new()
    };

    // Write the shell script to a temp file
    // 使用 exec 替换当前 shell 进程，trap 确保临时文件被清理
    let script_content = format!(
        r#"#!/bin/bash
trap 'rm -f "{script_file}" {cleanup}' EXIT
{cd_command}
exec {launch_command}
"#,
        cd_command = cd_command,
        launch_command = launch_command,
        cleanup = if cleanup_command.is_empty() {
            String::new()
        } else {
            format!(r#"; "{}""#, cleanup_command)
        },
        script_file = script_file.display()
    );

    std::fs::write(&script_file, &script_content).map_err(|e| format!("写入启动脚本失败: {e}"))?;

    // Make script executable
    std::fs::set_permissions(&script_file, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("设置脚本权限失败: {e}"))?;

    // Try the preferred terminal first, fall back to Terminal.app if it fails
    // Note: Kitty doesn't need the -e flag, others do
    let result = match terminal {
        "iterm2" => launch_macos_iterm2(&script_file),
        "alacritty" => launch_macos_open_app("Alacritty", &script_file, true),
        "kitty" => launch_macos_open_app("kitty", &script_file, false),
        "ghostty" => launch_macos_open_app("Ghostty", &script_file, true),
        _ => launch_macos_terminal_app(&script_file, working_dir), // "terminal" or default
    };

    // If preferred terminal fails and it's not the default, try Terminal.app as fallback
    if result.is_err() && terminal != "terminal" {
        log::warn!(
            "首选终端 {} 启动失败，回退到 Terminal.app: {:?}",
            terminal,
            result.as_ref().err()
        );
        return launch_macos_terminal_app(&script_file, working_dir);
    }

    result
}

/// macOS: Terminal.app
#[cfg(target_os = "macos")]
fn launch_macos_terminal_app(
    script_file: &std::path::Path,
    _working_dir: Option<&std::path::Path>,
) -> Result<(), String> {
    use std::process::Command;

    let applescript = format!(
        r#"tell application "Terminal"
    activate
    do script "bash '{}'"
end tell"#,
        script_file.display()
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&applescript)
        .output()
        .map_err(|e| format!("执行 osascript 失败: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Terminal.app 执行失败 (exit code: {:?}): {}",
            output.status.code(),
            stderr
        ));
    }

    Ok(())
}

/// macOS: iTerm2
#[cfg(target_os = "macos")]
fn launch_macos_iterm2(script_file: &std::path::Path) -> Result<(), String> {
    use std::process::Command;

    let applescript = format!(
        r#"tell application "iTerm"
    activate
    tell current window
        create tab with default profile
        tell current session
            write text "bash '{}'"
        end tell
    end tell
end tell"#,
        script_file.display()
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&applescript)
        .output()
        .map_err(|e| format!("执行 osascript 失败: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "iTerm2 执行失败 (exit code: {:?}): {}",
            output.status.code(),
            stderr
        ));
    }

    Ok(())
}

/// macOS: 使用 open -a 启动支持 --args 参数的终端（Alacritty/Kitty/Ghostty）
#[cfg(target_os = "macos")]
fn launch_macos_open_app(
    app_name: &str,
    script_file: &std::path::Path,
    use_e_flag: bool,
) -> Result<(), String> {
    use std::process::Command;

    let mut cmd = Command::new("open");
    cmd.arg("-a").arg(app_name).arg("--args");

    if use_e_flag {
        cmd.arg("-e");
    }
    cmd.arg("bash").arg(script_file);

    let output = cmd
        .output()
        .map_err(|e| format!("启动 {} 失败: {e}", app_name))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{} 启动失败 (exit code: {:?}): {}",
            app_name,
            output.status.code(),
            stderr
        ));
    }

    Ok(())
}

/// Linux: 根据用户首选终端启动
#[cfg(target_os = "linux")]
fn launch_linux_terminal(
    cli_command: &str,
    config_file: &Option<std::path::PathBuf>,
    working_dir: Option<&std::path::Path>,
    app_type: &AppType,
) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let preferred = crate::settings::get_preferred_terminal();

    // Default terminal list with their arguments
    let default_terminals = [
        ("gnome-terminal", vec!["--"]),
        ("konsole", vec!["-e"]),
        ("xfce4-terminal", vec!["-e"]),
        ("mate-terminal", vec!["--"]),
        ("lxterminal", vec!["-e"]),
        ("alacritty", vec!["-e"]),
        ("kitty", vec!["-e"]),
        ("ghostty", vec!["-e"]),
    ];

    // Create temp script file
    let temp_dir = std::env::temp_dir();
    let script_file = temp_dir.join(format!("cc_switch_launcher_{}.sh", std::process::id()));

    // 构建工作目录的 cd 命令（如果提供）
    let cd_command = if let Some(dir) = working_dir {
        let dir_path = dir.to_string_lossy();
        format!(r#"cd "{dir_path}""#)
    } else {
        String::new()
    };

    // 根据应用类型构建启动命令
    let launch_command = if *app_type == AppType::Claude {
        // Claude 需要 --settings 参数
        let config_path = config_file.as_ref().unwrap().to_string_lossy();
        format!(r#"{cli_command} --settings "{config_path}""#)
    } else {
        // 其他 CLI 直接启动
        cli_command.to_string()
    };

    // 清理命令（Claude 需要清理配置文件）
    let cleanup_command = if *app_type == AppType::Claude {
        let config_path = config_file.as_ref().unwrap().to_string_lossy();
        format!(r#"rm -f "{config_path}""#)
    } else {
        String::new()
    };

    // 使用 exec 替换当前 shell 进程，trap 确保临时文件被清理
    let script_content = format!(
        r#"#!/bin/bash
trap 'rm -f "{script_file}" {cleanup}' EXIT
{cd_command}
exec {launch_command}
"#,
        cd_command = cd_command,
        launch_command = launch_command,
        cleanup = if cleanup_command.is_empty() {
            String::new()
        } else {
            format!(r#"; "{}""#, cleanup_command)
        },
        script_file = script_file.display()
    );

    std::fs::write(&script_file, &script_content).map_err(|e| format!("写入启动脚本失败: {e}"))?;

    std::fs::set_permissions(&script_file, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("设置脚本权限失败: {e}"))?;

    // Build terminal list: preferred terminal first (if specified), then defaults
    let terminals_to_try: Vec<(&str, Vec<&str>)> = if let Some(ref pref) = preferred {
        // Find the preferred terminal's args from default list
        let pref_args = default_terminals
            .iter()
            .find(|(name, _)| *name == pref.as_str())
            .map(|(_, args)| args.iter().map(|s| *s).collect::<Vec<&str>>())
            .unwrap_or_else(|| vec!["-e"]); // Default args for unknown terminals

        let mut list = vec![(pref.as_str(), pref_args)];
        // Add remaining terminals as fallbacks
        for (name, args) in &default_terminals {
            if *name != pref.as_str() {
                list.push((*name, args.iter().map(|s| *s).collect()));
            }
        }
        list
    } else {
        default_terminals
            .iter()
            .map(|(name, args)| (*name, args.iter().map(|s| *s).collect()))
            .collect()
    };

    let mut last_error = String::from("未找到可用的终端");

    for (terminal, args) in terminals_to_try {
        // Check if terminal exists in common paths
        let terminal_exists = std::path::Path::new(&format!("/usr/bin/{}", terminal)).exists()
            || std::path::Path::new(&format!("/bin/{}", terminal)).exists()
            || std::path::Path::new(&format!("/usr/local/bin/{}", terminal)).exists()
            || which_command(terminal);

        if terminal_exists {
            let result = Command::new(terminal)
                .args(&args)
                .arg("bash")
                .arg(script_file.to_string_lossy().as_ref())
                .spawn();

            match result {
                Ok(_) => return Ok(()),
                Err(e) => {
                    last_error = format!("执行 {} 失败: {}", terminal, e);
                }
            }
        }
    }

    // Clean up on failure
    let _ = std::fs::remove_file(&script_file);
    if let Some(ref config) = config_file {
        let _ = std::fs::remove_file(config);
    }
    Err(last_error)
}

/// Check if a command exists using `which`
#[cfg(target_os = "linux")]
fn which_command(cmd: &str) -> bool {
    use std::process::Command;
    Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 查找系统中的 git-bash 路径
#[cfg(target_os = "windows")]
fn find_git_bash() -> Option<String> {
    use std::process::Command;

    // 常见的 Git 安装路径
    let common_paths = vec![
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
        r"C:\Git\bin\bash.exe",
    ];

    // 首先检查环境变量
    if let Ok(bash_path) = std::env::var("CLAUDE_CODE_GIT_BASH_PATH") {
        if std::path::Path::new(&bash_path).exists() {
            log::info!("[GitBash] 从环境变量找到: {}", bash_path);
            return Some(bash_path);
        }
    }

    // 检查常见安装路径
    for path in common_paths {
        if std::path::Path::new(path).exists() {
            log::info!("[GitBash] 从常见路径找到: {}", path);
            return Some(path.to_string());
        }
    }

    // 尝试通过 where 命令查找
    if let Ok(output) = Command::new("where")
        .args(&["bash.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        if output.status.success() {
            let paths = String::from_utf8_lossy(&output.stdout);
            if let Some(first_line) = paths.lines().next() {
                let bash_path = first_line.trim().to_string();
                log::info!("[GitBash] 通过 where 命令找到: {}", bash_path);
                return Some(bash_path);
            }
        }
    }

    log::warn!("[GitBash] 未找到 git-bash");
    None
}

/// Windows: 根据用户首选终端启动
#[cfg(target_os = "windows")]
fn launch_windows_terminal(
    cli_command: &str,
    config_file: &Option<std::path::PathBuf>,
    temp_dir: &std::path::Path,
    _provider_id: &str,
    working_dir: Option<&std::path::Path>,
    app_type: &AppType,
) -> Result<(), String> {
    let preferred = crate::settings::get_preferred_terminal();
    let terminal = preferred.as_deref().unwrap_or("cmd");

    let bat_file = temp_dir.join(format!(
        "cc_switch_{}_{}.bat",
        cli_command,
        std::process::id()
    ));

    // 构建工作目录的 cd 命令（如果提供）
    let cd_command = if let Some(dir) = working_dir {
        let dir_path = dir.to_string_lossy().replace('&', "^&");
        format!(r#"cd /d "{}""#, dir_path)
    } else {
        String::new()
    };

    // 获取 CLI 命令（Windows 上 npm 安装的 CLI 通常是 .cmd 文件）
    let cli_command_exe = if cfg!(windows) {
        // Windows 上优先尝试 .cmd 扩展名
        format!("{}.cmd", cli_command)
    } else {
        cli_command.to_string()
    };

    // 根据应用类型构建启动命令
    let (launch_line, cleanup_lines) = if *app_type == AppType::Claude {
        // Claude 需要 --settings 参数
        let config_path = config_file
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .replace('&', "^&");
        let launch_line = format!(r#"{} --settings "{}""#, cli_command_exe, config_path);
        let cleanup_lines = format!(r#"del "{}" >nul 2>&1"#, config_path);
        (launch_line, cleanup_lines)
    } else {
        // 其他 CLI 直接启动
        (cli_command_exe.clone(), String::new())
    };

    // 查找 git-bash 并设置环境变量（Claude Code 需要）
    let git_bash_set = if *app_type == AppType::Claude {
        if let Some(bash_path) = find_git_bash() {
            format!(r#"set "CLAUDE_CODE_GIT_BASH_PATH={}""#, bash_path)
        } else {
            "// git-bash not found, Claude may fail".to_string()
        }
    } else {
        String::new()
    };

    // 异步启动 CLI，不等待命令完成
    // Note: Using English only to avoid encoding issues on Windows (bat files use system codepage)
    let content = format!(
        r#"@echo off
{}
echo Starting {}...
{}
echo Running: {}
{}
{}
del "%~f0" >nul 2>&1
"#,
        git_bash_set,
        cli_command,
        cd_command,
        launch_line,
        launch_line,  // Direct execution without 'call' for async launch
        if cleanup_lines.is_empty() {
            String::new()
        } else {
            cleanup_lines.clone()
        }
    );

    std::fs::write(&bat_file, &content).map_err(|e| format!("写入批处理文件失败: {e}"))?;

    // 添加调试日志
    log::info!("[Terminal] 批处理文件路径: {}", bat_file.display());
    log::info!("[Terminal] CLI 命令: {}", cli_command_exe);
    log::info!("[Terminal] 启动行: {}", launch_line);
    log::info!("[Terminal] 工作目录: {:?}", working_dir);
    log::info!("[Terminal] 终端类型: {}", terminal);

    let bat_path = bat_file.to_string_lossy();
    let ps_cmd = format!("& '{}'", bat_path);

    // Try the preferred terminal first
    let result = match terminal {
        "powershell" => run_windows_start_command(
            &["powershell", "-NoExit", "-Command", &ps_cmd],
            "PowerShell",
            working_dir,
        ),
        "wt" => run_windows_start_command(
            &["wt", "cmd", "/K", &bat_path],
            "Windows Terminal",
            working_dir,
        ),
        _ => run_windows_start_command(&["cmd", "/K", &bat_path], "cmd", working_dir), // "cmd" or default
    };

    // If preferred terminal fails and it's not the default, try cmd as fallback
    if result.is_err() && terminal != "cmd" {
        log::warn!(
            "首选终端 {} 启动失败，回退到 cmd: {:?}",
            terminal,
            result.as_ref().err()
        );
        return run_windows_start_command(&["cmd", "/K", &bat_path], "cmd", working_dir);
    }

    result
}

/// Windows: Run a start command with common error handling
#[cfg(target_os = "windows")]
fn run_windows_start_command(
    args: &[&str],
    terminal_name: &str,
    _working_dir: Option<&std::path::Path>,
) -> Result<(), String> {
    use std::process::Command;

    let mut full_args = vec!["/C", "start"];
    full_args.extend(args);

    let output = Command::new("cmd")
        .args(&full_args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("启动 {} 失败: {e}", terminal_name))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{} 启动失败 (exit code: {:?}): {}",
            terminal_name,
            output.status.code(),
            stderr
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    #[test]
    fn test_find_git_bash_with_env_var() {
        // 设置测试环境变量
        let test_bash_path = r"C:\Test\Git\bin\bash.exe";
        std::env::set_var("CLAUDE_CODE_GIT_BASH_PATH", test_bash_path);

        // 由于测试环境可能没有这个路径，我们只验证函数会读取环境变量
        // 实际的路径检查在生产环境中进行
        let result = find_git_bash();

        // 清理环境变量
        std::env::remove_var("CLAUDE_CODE_GIT_BASH_PATH");

        // 如果路径不存在（测试环境），函数会尝试其他方法
        // 这个测试主要验证函数不会崩溃
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_find_git_bash_no_env_var() {
        // 确保环境变量未设置
        std::env::remove_var("CLAUDE_CODE_GIT_BASH_PATH");

        // 测试在没有环境变量时的行为
        let result = find_git_bash();

        // 这个测试主要验证函数不会崩溃
        // 在实际环境中可能会找到 git-bash
    }
}
