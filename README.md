# Cargo 项目清理工具

> Cargo Project Cleanup Utility

一个简单实用的 Rust 命令行工具，用于自动扫描并清理工作目录中的 Cargo 项目 `target` 目录，释放磁盘空间。

A simple and practical Rust CLI tool that automatically scans and cleans up Cargo project `target` directories in your workspace to free up disk space.

---

## 功能特性

### 主要功能

- **自动扫描**：从程序所在目录开始递归遍历，自动查找所有 Cargo 项目
- **智能识别**：仅识别同时包含 `Cargo.toml` 和 `target` 目录的有效项目
- **交互清理**：对每个项目询问用户是否执行清理操作
- **批量操作**：支持单选模式或全局模式，满足不同需求
- **错误处理**：优雅处理权限拒绝等错误，不中断扫描流程
- **统计报告**：清理完成后显示清理/跳过项目数量统计

### 操作选项

| 按键 | 功能 |
|------|------|
| `y` | 对当前项目执行 `cargo clean` |
| `n` | 跳过当前项目，继续处理下一个 |
| `s` | 对所有后续项目执行 clean（全选模式） |
| `q` | 立即退出程序 |

### 技术特点

- 使用栈结构实现非递归深度优先遍历，避免栈溢出
- 彩色输出带 emoji 符号，视觉友好
- 详细的错误信息和上下文提示
- 支持中文和英文界面输出

---

## 安装方法

### 方式一：源码编译（推荐）

```bash
# 克隆或进入项目目录
cd clean_cargo_projects

# 编译项目
cargo build --release

# 编译完成后，可执行文件位于：
# target/release/clean_cargo_projects.exe (Windows)
# target/release/clean_cargo_projects (Linux/macOS)
```

### 方式二：下载预编译二进制文件

从 [GitHub Releases](https://github.com/yourusername/clean_cargo_projects/releases) 下载对应平台的预编译版本。

---

## 使用方法

### 基本用法

1. 将编译好的可执行文件放置在您想要扫描的目录（或其父目录）
2. 运行可执行文件
3. 程序会自动从可执行文件所在目录开始扫描所有子目录
4. 对每个找到的 Cargo 项目进行交互式清理

### 示例输出

```
遍历目录: C:/Users/example/projects
============================================================
提示: y=执行 clean, n=跳过, s=全部执行, q=全部退出
============================================================

⏳ [遍历] projects/
  └── ⏳ [遍历] my_project/
      └── ✓ 找到 Cargo.toml + target/

是否执行 'cargo clean'? [y/n/s/q]: y

正在执行 cargo clean...
✓ 清理成功: C:/Users/example/projects/my_project

  └── ⏭️ [跳过] another_project/ (无权限访问)

[中止] 用户选择退出

============================================================
遍历完成!
  ✓ 清理完成: 1 个项目
  ○ 跳过: 1 个项目
============================================================
```

---

## 项目结构

```
clean_cargo_projects/
├── src/
│   └── main.rs          # 程序入口和核心逻辑
├── Cargo.toml           # 项目配置文件
├── Cargo.lock           # 依赖锁定文件
└── README.md           # 说明文档
```

---

## 依赖项

本项目使用以下 Rust 依赖：

| 依赖 | 版本 | 用途 |
|------|------|------|
| `walkdir` | 2.x | 目录遍历 |
| `dialoguer` | 0.11 | 交互式用户输入 |
| `anyhow` | 1.0 | 错误处理 |

---

## 构建要求

- Rust 1.56.0 或更高版本
- Cargo 包管理器

```bash
# 检查 Rust 版本
rustc --version
cargo --version
```

---

## 工作原理

1. **获取目录**：程序首先确定自身所在的目录作为扫描起点
2. **遍历目录**：使用栈结构进行深度优先遍历，扫描所有子目录
3. **识别项目**：检查每个目录是否同时包含 `Cargo.toml` 和 `target` 目录
4. **交互确认**：对每个有效项目提示用户选择操作
5. **执行清理**：根据用户选择执行 `cargo clean` 命令
6. **输出统计**：显示清理结果统计

---

## 常见问题

### Q: 程序会清理所有 Cargo 项目吗？

A: 不会。程序只会清理同时存在 `Cargo.toml` 和 `target` 目录的项目，确保只处理有效的 Cargo 项目。

### Q: 可以指定扫描目录吗？

A: 当前版本从程序所在目录开始扫描。如需指定其他目录，可将程序复制到目标目录后运行。

### Q: 误删了重要文件怎么办？

A: `cargo clean` 只会删除 `target` 目录中的编译产物（`.exe`、`.rlib` 等），不会影响源代码。

### Q: 支持 Linux/macOS 吗？

A: 支持。本项目使用标准 Rust 库，可跨平台编译运行。

---

## 许可证

本项目采用 MIT 许可证开源。

---

## 贡献

欢迎提交 Issue 和 Pull Request！

---

## 更新日志

### v0.1.0 (2025-02-05)

- 初始版本发布
- 实现基础扫描和清理功能
- 支持交互式操作
- 支持批量操作模式
