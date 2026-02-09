# Windows 右键菜单级联菜单实现（HKCU）

## 问题
Windows 右键菜单的级联菜单（子菜单）在 HKCU 下无法展开，只显示箭头但点击无反应。

## 根本原因
1. **HKCU 下的 CommandStore 不被识别** - SubCommands + CommandStore 方式只适用于 HKLM
2. **嵌套 shell 结构不被支持** - 直接在主菜单下创建 `shell` 子键在 HKCU 下无法展开
3. **ExtendedSubCommandsKey 在 HKCU 下无效** - Microsoft 文档方法在 HKCU 下不工作

## 正确方案（HKCU 专用）

根据 [Stack Overflow - Make context menu submenu per user](https://stackoverflow.com/questions/59738057/make-context-menu-submenu-per-user-in-windows-explorer)：

### 注册表结构
```
HKCU\Software\Classes\Directory\Background\shell\CLI Switch
  MUIVerb = "CLI Switch"
  SubCommands = ""  ← 关键：必须设置为空字符串
  (Default) = (值 not set)  ← 不能有值

HKCU\Software\Classes\Directory\Background\shell\CLI Switch\Shell\
  codex\                     ← 子菜单项直接放这里
    @ = "Open Codex Terminal"
    command\
      @ = "exe --app codex --dir %V"

  gemini\
    @ = "Open Gemini Terminal"
    command\
      @ = "exe --app gemini --dir %V"
```

### 关键点
1. **`SubCommands` 必须设置为空字符串 `""`** - 不是不设置，而是设置为空
2. **子菜单项放在 `主菜单\Shell\` 下** - 不使用 CommandStore
3. **`Directory\Background` 使用 `%V`** - 获取当前目录路径
4. **`Directory` 使用 `%1`** - 获取被点击项路径

### 代码实现
见 `src-tauri/src/commands/context_menu.rs`

### 参考资料
- [Stack Overflow - Make context menu submenu per user in Windows Explorer](https://stackoverflow.com/questions/59738057/make-context-menu-submenu-per-user-in-windows-explorer)
- [Microsoft - Creating Cascading Menus with the SubCommands Registry Entry](https://learn.microsoft.com/en-us/windows/win32/shell/how-to--create-cascading-menus-with-the-subcommands-registry-entry)
