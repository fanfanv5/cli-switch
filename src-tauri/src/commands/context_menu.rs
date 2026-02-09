//! Windows 文件夹右键菜单集成
//!
//! 在 Windows 资源管理器的文件夹右键菜单中添加"打开 CLI 终端"选项
//! 支持两种模式：
//! 1. 默认终端：不带供应商配置，直接启动 CLI
//! 2. 供应商终端：使用特定供应商配置启动 CLI

#[cfg(target_os = "windows")]
use winreg::enums::*;
#[cfg(target_os = "windows")]
use winreg::RegKey;
#[cfg(target_os = "windows")]
use crate::store::AppState;
#[cfg(target_os = "windows")]
use tauri::Manager;

/// 检查当前进程是否以管理员身份运行
#[cfg(target_os = "windows")]
fn is_elevated() -> bool {
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};

    unsafe {
        let process_handle = GetCurrentProcess();
        let mut token_handle = HANDLE::default();
        if OpenProcessToken(process_handle, TOKEN_QUERY, &mut token_handle).is_err() {
            return false;
        }

        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut return_length = 0;
        GetTokenInformation(
            token_handle,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut return_length,
        ).is_ok() && elevation.TokenIsElevated != 0
    }
}

/// Windows 右键菜单注册表基路径
/// HKEY_CURRENT_USER\Software\Classes\Directory\Background\shell\CLI Switch
#[cfg(target_os = "windows")]
const MENU_REGISTRY_KEY: &str = r"Software\Classes\Directory\Background\shell\CLI Switch";

/// 注册 Windows 文件夹右键菜单
///
/// 创建结构：
/// - Directory\shell\CLI Switch\shell\... (点击文件夹时)
/// - Directory\Background\shell\CLI Switch\shell\... (点击文件夹空白处时)
/// 所有菜单项折叠到 CLI Switch 子菜单中
/// - 默认终端："Open {App} Terminal"
/// - 供应商终端："Open {App} - {Provider} Terminal"
#[cfg(target_os = "windows")]
#[tauri::command]
pub async fn register_context_menu(
    app: tauri::AppHandle,
) -> Result<(), String> {
    // 获取 exe 路径
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("获取 exe 路径失败: {}", e))?;

    log::info!("开始注册右键菜单，exe 路径: {}", exe_path.display());

    // 获取应用状态以访问数据库
    let state = app
        .try_state::<AppState>()
        .ok_or("无法获取应用状态")?;
    log::info!("获取应用状态成功");

    // 只注册到空白处右键菜单 (Directory\Background\shell)
    let registry_path = r"Software\Classes\Directory\Background\shell\CLI Switch";
    log::info!("注册到: {}", registry_path);
    register_menus_at_path(registry_path, &exe_path, &state, true)?;

    log::info!("右键菜单注册成功");
    Ok(())
}

/// 在指定注册表路径注册菜单
/// is_background: 是否为 Directory\Background（空白处右键），需要用 %V 而非 %1
#[cfg(target_os = "windows")]
fn register_menus_at_path(
    registry_path: &str,
    exe_path: &std::path::Path,
    state: &AppState,
    is_background: bool,
) -> Result<(), String> {
    let base_key = RegKey::predef(HKEY_CURRENT_USER);
    log::info!("打开注册表基路径: HKEY_CURRENT_USER\\{}", registry_path);
    let (menu_key, _) = base_key
        .create_subkey(registry_path)
        .map_err(|e| format!("打开注册表失败: {}", e))?;
    log::info!("注册表主键创建成功");

    // 设置主菜单项 - HKCU 下级联菜单的正确方式
    menu_key
        .set_value("MUIVerb", &"CLI Switch")
        .map_err(|e| format!("设置菜单名称失败: {}", e))?;
    menu_key
        .set_value("Icon", &exe_path.to_string_lossy().to_string())
        .map_err(|e| format!("设置图标失败: {}", e))?;
    // 关键：设置 subcommands 为空字符串，使菜单成为级联菜单
    menu_key
        .set_value("SubCommands", &"")
        .map_err(|e| format!("设置 SubCommands 失败: {}", e))?;
    // 显式删除 (Default) 值
    let _ = menu_key.delete_value("");
    log::info!("主菜单项设置完成");

    // 创建 Shell 子键（子菜单项放在这里，不是 CommandStore）
    let (shell_key, _) = menu_key
        .create_subkey("Shell")
        .map_err(|e| format!("创建 Shell 子项失败: {}", e))?;

    // 清空现有菜单项
    for name in shell_key.enum_keys().flatten() {
        let _ = shell_key.delete_subkey_all(&name);
    }
    log::info!("已清空现有菜单项");

    // Claude: 按供应商列表
    let providers = state
        .db
        .get_all_providers("claude")
        .map_err(|e| format!("获取 Claude 供应商列表失败: {}", e))?;

    for (provider_id, provider) in providers {
        let verb = format!("ccswitch.claude.{}", sanitize_registry_key(&provider_id));
        let suffix = if let Some(notes) = &provider.notes {
            format!("{} - {}", provider.name, notes)
        } else {
            provider.name.clone()
        };
        let display_name = format!("Open Claude - {}", suffix);
        register_shell_verb(&shell_key, &verb, &display_name, exe_path, "claude", Some(&provider_id), is_background)?;
    }

    // Codex/Gemini/OpenCode: 直接唤起
    for app_type in ["codex", "gemini", "opencode"] {
        let verb = format!("ccswitch.{}", app_type);
        let display_name = format!("Open {} Terminal", get_app_display_name(app_type));
        register_shell_verb(&shell_key, &verb, &display_name, exe_path, app_type, None, is_background)?;
    }

    Ok(())
}

/// 为单个应用类型注册菜单项
/// Claude: 按供应商列表加载子项
/// Codex/Gemini/OpenCode: 直接唤起，不显示供应商子项
#[cfg(target_os = "windows")]
fn register_app_menus(
    menu_key: &RegKey,
    exe_path: &std::path::Path,
    app_type: &str,
    state: &AppState,
    is_background: bool,
) -> Result<(), String> {
    let display_name = get_app_display_name(app_type);

    // Claude 需要供应商子菜单，其他直接唤起
    if app_type == "claude" {
        // 注册 Claude 供应商列表
        let providers = state
            .db
            .get_all_providers(app_type)
            .map_err(|e| format!("获取供应商列表失败: {}", e))?;

        log::debug!("找到 {} 个 Claude 供应商", providers.len());
        for (provider_id, provider) in providers {
            // 构建菜单显示名称 - 使用英文避免编码问题
            let suffix = if let Some(notes) = &provider.notes {
                format!("{} - {}", provider.name, notes)
            } else {
                provider.name.clone()
            };
            let key_name = format!("claude_{}", sanitize_registry_key(&provider_id));
            let display_name = format!("Open Claude - {}", suffix);

            log::debug!("注册 Claude 供应商: {} -> {}", key_name, display_name);
            register_menu_item(
                menu_key,
                &key_name,
                &display_name,
                exe_path,
                app_type,
                Some(&provider_id),
                is_background,
            )?;
        }
    } else {
        // Codex/Gemini/OpenCode: 直接唤起，不显示供应商子项
        let key_name = app_type;
        let display_name = format!("Open {} Terminal", display_name);

        log::debug!("注册直接唤起: {} -> {}", key_name, display_name);
        register_menu_item(
            menu_key,
            key_name,
            &display_name,
            exe_path,
            app_type,
            None,
            is_background,
        )?;
    }

    Ok(())
}

/// 在 Shell 下注册单个动词（HKCU 级联菜单方式）
#[cfg(target_os = "windows")]
fn register_shell_verb(
    shell_key: &RegKey,
    verb: &str,
    display_name: &str,
    exe_path: &std::path::Path,
    app_type: &str,
    provider_id: Option<&str>,
    is_background: bool,
) -> Result<(), String> {
    // 创建动词子键
    let (verb_key, _) = shell_key
        .create_subkey(verb)
        .map_err(|e| format!("创建动词子键失败 [{}]: {}", verb, e))?;

    // 设置显示名称
    verb_key
        .set_value("", &display_name)
        .map_err(|e| format!("设置动词名称失败 [{}]: {}", verb, e))?;
    verb_key
        .set_value("MUIVerb", &display_name)
        .ok();

    // 设置图标
    verb_key
        .set_value("Icon", &exe_path.to_string_lossy().to_string())
        .ok();

    // Directory\Background 使用 %V，其他使用 %1
    let dir_param = if is_background { "%V" } else { "%1" };

    // 构建命令
    let command = if let Some(pid) = provider_id {
        format!(
            "\"{}\" --open-terminal --app {} --dir \"{}\" --provider-id \"{}\"",
            exe_path.display(),
            app_type,
            dir_param,
            pid
        )
    } else {
        format!(
            "\"{}\" --open-terminal --app {} --dir \"{}\"",
            exe_path.display(),
            app_type,
            dir_param
        )
    };

    // 创建 command 子键
    let (cmd_key, _) = verb_key
        .create_subkey("command")
        .map_err(|e| format!("创建命令项失败 [{}]: {}", verb, e))?;

    cmd_key
        .set_value("", &command)
        .map_err(|e| format!("设置命令失败 [{}]: {}", verb, e))?;

    log::debug!("注册 Shell 动词: {} -> {}", verb, display_name);
    Ok(())
}

/// 在 CommandStore 中注册单个动词（已弃用，HKCU 下不支持）
/// CommandStore 路径: HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\
#[cfg(target_os = "windows")]
fn register_command_store_verb(
    verb: &str,
    display_name: &str,
    exe_path: &std::path::Path,
    app_type: &str,
    provider_id: Option<&str>,
    is_background: bool,
) -> Result<(), String> {
    let command_store_path = format!(
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\{}",
        verb
    );

    let base_key = RegKey::predef(HKEY_CURRENT_USER);
    let (verb_key, _) = base_key
        .create_subkey(&command_store_path)
        .map_err(|e| format!("创建 CommandStore 动词失败 [{}]: {}", verb, e))?;

    // 设置显示名称
    verb_key
        .set_value("", &display_name)
        .map_err(|e| format!("设置动词名称失败 [{}]: {}", verb, e))?;
    verb_key
        .set_value("MUIVerb", &display_name)
        .ok();

    // 设置图标
    verb_key
        .set_value("Icon", &exe_path.to_string_lossy().to_string())
        .ok();

    // Directory\Background 使用 %V，其他使用 %1
    let dir_param = if is_background { "%V" } else { "%1" };

    // 构建命令
    let command = if let Some(pid) = provider_id {
        format!(
            "\"{}\" --open-terminal --app {} --dir \"{}\" --provider-id \"{}\"",
            exe_path.display(),
            app_type,
            dir_param,
            pid
        )
    } else {
        format!(
            "\"{}\" --open-terminal --app {} --dir \"{}\"",
            exe_path.display(),
            app_type,
            dir_param
        )
    };

    // 创建 command 子键
    let (cmd_key, _) = verb_key
        .create_subkey("command")
        .map_err(|e| format!("创建命令项失败 [{}]: {}", verb, e))?;

    cmd_key
        .set_value("", &command)
        .map_err(|e| format!("设置命令失败 [{}]: {}", verb, e))?;

    log::debug!("注册 CommandStore 动词: {} -> {}", verb, display_name);
    Ok(())
}

/// 注册单个菜单项（已弃用，保留用于 register_context_menu_hidden）
#[cfg(target_os = "windows")]
fn register_menu_item(
    menu_key: &RegKey,
    key_name: &str,
    display_name: &str,
    exe_path: &std::path::Path,
    app_type: &str,
    provider_id: Option<&str>,
    is_background: bool,
) -> Result<(), String> {
    // 创建菜单项键（解构元组）
    let (item_key, _) = menu_key
        .create_subkey(key_name)
        .map_err(|e| format!("创建菜单项失败 [{}]: {}", key_name, e))?;

    // 设置显示名称（同时设置默认值和 MUIVerb）
    item_key
        .set_value("", &display_name)
        .map_err(|e| format!("设置菜单名称失败 [{}]: {}", key_name, e))?;
    item_key
        .set_value("MUIVerb", &display_name)
        .ok();

    // 设置图标
    item_key
        .set_value("Icon", &exe_path.to_string_lossy().to_string())
        .ok();

    // Directory\Background 使用 %V，其他使用 %1
    let dir_param = if is_background { "%V" } else { "%1" };

    // 构建命令
    let command = if let Some(pid) = provider_id {
        format!(
            "\"{}\" --open-terminal --app {} --dir \"{}\" --provider-id \"{}\"",
            exe_path.display(),
            app_type,
            dir_param,
            pid
        )
    } else {
        format!(
            "\"{}\" --open-terminal --app {} --dir \"{}\"",
            exe_path.display(),
            app_type,
            dir_param
        )
    };

    // 创建 command 子键（解构元组）
    let (cmd_key, _) = item_key
        .create_subkey("command")
        .map_err(|e| format!("创建命令项失败 [{}]: {}", key_name, e))?;

    cmd_key
        .set_value("", &command)
        .map_err(|e| format!("设置命令失败 [{}]: {}", key_name, e))?;

    log::debug!("注册菜单项: {} -> {}", key_name, display_name);
    Ok(())
}

/// 注销 Windows 文件夹右键菜单
#[cfg(target_os = "windows")]
#[tauri::command]
pub async fn unregister_context_menu() -> Result<(), String> {
    log::info!("开始注销右键菜单");

    let base_key = RegKey::predef(HKEY_CURRENT_USER);

    // 删除 Directory\shell 的旧数据（不再使用）
    let _ = base_key.delete_subkey_all(r"Software\Classes\Directory\shell\CLI Switch");
    // 删除 Directory\Background\shell 的注册表项
    log::info!("删除注册表项: Software\\Classes\\Directory\\Background\\shell\\CLI Switch");
    let _ = base_key.delete_subkey_all(r"Software\Classes\Directory\Background\shell\CLI Switch");

    log::info!("右键菜单注销成功");
    Ok(())
}

/// 获取应用显示名称
#[cfg(target_os = "windows")]
fn get_app_display_name(app_type: &str) -> String {
    match app_type {
        "claude" => "Claude".to_string(),
        "codex" => "Codex".to_string(),
        "gemini" => "Gemini".to_string(),
        "opencode" => "OpenCode".to_string(),
        _ => app_type.to_string(),
    }
}

/// 清理注册表键名中的非法字符
#[cfg(target_os = "windows")]
fn sanitize_registry_key(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

/// 检查右键菜单是否已注册
/// 申请管理员权限并重新执行注册
///
/// 创建临时脚本并使用 PowerShell 以管理员身份运行
#[cfg(target_os = "windows")]
fn request_elevated_registration() -> Result<(), String> {
    use std::io::Write;
    use std::process::Command;

    let exe_path = std::env::current_exe()
        .map_err(|e| format!("获取 exe 路径失败: {}", e))?;

    // 创建临时 PowerShell 脚本
    let temp_dir = std::env::temp_dir();
    let script_path = temp_dir.join("cc_switch_register_menu.ps1");

    let script_content = format!(
        r#"
Start-Process -FilePath "{}" -ArgumentList "--register-context-menu-hidden" -Verb RunAs
"#,
        exe_path.display()
    );

    let mut file = std::fs::File::create(&script_path)
        .map_err(|e| format!("创建脚本文件失败: {}", e))?;
    file.write_all(script_content.as_bytes())
        .map_err(|e| format!("写入脚本失败: {}", e))?;

    // 使用 PowerShell 执行脚本（会弹出 UAC）
    let result = Command::new("powershell.exe")
        .args(["-ExecutionPolicy", "Bypass", "-File", &script_path.to_string_lossy()])
        .spawn();

    match result {
        Ok(_) => {
            // 脚本已启动，删除临时脚本
            let _ = std::fs::remove_file(&script_path);
            Err("UAC 已弹出，请在弹出的窗口中确认注册".to_string())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&script_path);
            Err(format!("启动管理员权限申请失败: {}", e))
        }
    }
}

/// 隐藏模式的注册命令（用于管理员权限脚本调用）
///
/// 此命令不检查权限，直接执行注册
#[cfg(target_os = "windows")]
#[tauri::command]
pub async fn register_context_menu_hidden(
    app: tauri::AppHandle,
) -> Result<(), String> {
    // 直接执行注册，不检查权限
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("获取 exe 路径失败: {}", e))?;

    log::info!("开始注册右键菜单（管理员模式），exe 路径: {}", exe_path.display());

    // 获取应用状态以访问数据库
    let state = app
        .try_state::<AppState>()
        .ok_or("无法获取应用状态")?;

    // 使用相同的 Shell 方式注册（HKCU 下级联菜单）
    register_menus_at_path(MENU_REGISTRY_KEY, &exe_path, &state, true)?;

    log::info!("右键菜单注册成功");
    Ok(())
}

#[cfg(target_os = "windows")]
#[tauri::command]
pub async fn is_context_menu_registered() -> Result<bool, String> {
    let base_key = RegKey::predef(HKEY_CURRENT_USER);
    match base_key.open_subkey(MENU_REGISTRY_KEY) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

// 非 Windows 平台的空实现
#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub async fn register_context_menu(
    _app: tauri::AppHandle,
) -> Result<(), String> {
    Err("右键菜单功能仅支持 Windows 平台".to_string())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub async fn unregister_context_menu() -> Result<(), String> {
    Err("右键菜单功能仅支持 Windows 平台".to_string())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub async fn is_context_menu_registered() -> Result<bool, String> {
    Ok(false)
}
