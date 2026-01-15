# ForgeFFI

[![Rust](https://img.shields.io/badge/Rust-2024-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![FFI](https://img.shields.io/badge/FFI-cdylib%20%7C%20staticlib-5B8CFF)](https://doc.rust-lang.org/nomicon/ffi.html)
[![License](https://img.shields.io/badge/License-Apache--2.0%20OR%20MIT-2F9E44)](LICENSE-APACHE)

[English](README.en.md)

✨ 把 Rust 的性能、安全与工程化交付，像 SDK 一样带到任何语言。

ForgeFFI 是一套用 Rust 实现的通用工具库 + 生产级 FFI 交付管线，目标是让你无需手工维护复杂的跨平台构建脚本，也能稳定产出可分发的动态库/静态库与 C 头文件。

🚧 进度说明：目前仓库以“构建/交付管线 + 分层骨架”为主，模块能力正在逐步落地。

## 导航

- [✨ 你会得到什么](#你会得到什么)
- [🧩 模块与能力](#模块与能力)
- [🧭 架构一览](#架构一览)
- [🚀 快速上手](#快速上手)
- [📦 产物结构（dist）](#产物结构dist)
- [🌍 交叉编译与 all 构建](#交叉编译与-all-构建)
- [🧪 开发与质量检查](#开发与质量检查)
- [🤝 开源协作](#开源协作)
- [📄 许可证](#许可证)

## 你会得到什么

- 一条命令生成可分发产物：`.dll/.so/.dylib`、`.a/.lib`、`.h`
- 统一的产物目录结构：稳定落在 `dist/<target>/<profile>/...`，适配 CI/CD 与发布
- 模块化的能力组合：按 feature 选择 net/fs/sys，也可做聚合交付
- 交互式/非交互式构建入口：开发期用菜单，流水线用参数
- 可选 Zig 辅助交叉编译：尽量减少系统工具链差异

## 模块与能力

下面是模块划分与能力边界（会持续演进）。为避免“README 过度承诺”，这里标注当前状态：

- ✅ 已具备：构建/交付管线（xtask）、聚合 features、FFI 产物骨架、dist 目录结构
- 🚧 开发中：各模块的具体功能 API 与稳定的 FFI ABI

| 模块 | Rust crate | FFI crate | 状态 | 计划覆盖（示例方向） |
| --- | --- | --- | --- | --- |
| 基础能力 | `forgeffi-base` | - | 🚧 | 通用类型/错误码、跨平台兼容层、FFI 共享约定 |
| 网络 | `forgeffi-net` | `forgeffi-net-ffi` | 🚧 | Socket/TCP/UDP、地址解析、连接/超时/取消等基础网络能力 |
| 文件系统 | `forgeffi-fs` | `forgeffi-fs-ffi` | 🚧 | 文件/目录操作、遍历与元信息、跨平台路径与权限处理 |
| 系统 | `forgeffi-sys` | `forgeffi-sys-ffi` | 🚧 | 进程/环境/系统信息等跨平台系统能力 |
| Rust 聚合 | `forgeffi` | - | ✅ | 通过 features 组合 net/fs/sys/full |
| FFI 聚合 | - | `forgeffi-ffi` | ✅ | 通过 features 组合 net/fs/sys/full |

<details>
<summary>🎯 设计目标（点击展开）</summary>

- 🧱 分层清晰：实现模块与 FFI 产物分 crate 隔离，避免 unsafe 扩散
- 📦 可交付：稳定的 dist 目录结构与一键构建入口，便于 CI/CD
- 🔌 可扩展：模块化 feature gating，支持按需裁剪与聚合发布

</details>

## 架构一览

当前仓库已搭建好“模块实现 / 聚合入口 / FFI 产物 / 构建前端”的分层，具体能力实现仍在演进。

```text
                        ┌──────────────────────────┐
                        │          xtask           │
                        │  menu/build/zig/headers  │
                        └───────────┬──────────────┘
                                    │
                                    │  dist/<target>/<profile>/...
                                    ▼
┌──────────────────────────────────────────────────────────────────┐
│                           交付物（FFI）                           │
│  forgeffi-ffi (聚合 FFI) / forgeffi-*-ffi (模块 FFI)              │
│  crate-type = cdylib, staticlib  +  cbindgen 生成 .h              │
└──────────────────────────────────────────────────────────────────┘
                 ▲                                   ▲
                 │                                   │
                 │ 依赖 Rust 实现                     │ 依赖公共基础
                 │                                   │
┌──────────────────────────┐             ┌──────────────────────────┐
│      Rust 能力模块         │             │       forgeffi-base       │
│ forgeffi-net / fs / sys   │             │  跨模块复用的基础能力      │
└──────────────────────────┘             └──────────────────────────┘

┌──────────────────────────────────────────────────────────────────┐
│                           Rust 聚合入口                            │
│ forgeffi（通过 features 组合 net/fs/sys/full）                      │
└──────────────────────────────────────────────────────────────────┘
```

## 快速上手

### 1) 交互式构建（推荐）

```bash
cargo run -p xtask menu
# cargo xtask menu
```

可选择：profile、模式（模块/聚合、Rust/FFI）、产物类型（动态/静态）、目标平台（包含 `all（全部）`）、是否使用 zigbuild、是否生成头文件。

### 2) 非交互式构建

模块 FFI（以 `net` 为例，生成动态库 + 头文件）：

```bash
cargo xtask build \
  --mode module-ffi \
  --modules net \
  --profile release \
  --artifact cdylib \
  --headers=true
```

聚合 FFI（以 `full` 为例，生成动态库 + 头文件）：

```bash
cargo xtask build \
  --mode aggregate-ffi \
  --features full \
  --profile release \
  --artifact cdylib \
  --headers=true
```

仅下载并打印 Zig 路径（会缓存复用）：

```bash
cargo xtask zig --version 0.12.0
```

## 产物结构（dist）

FFI 构建会把产物复制到 `dist/`：

```text
dist/<target>/<debug|release>/<pkg>/<cdylib|staticlib>/...
dist/<target>/<debug|release>/<pkg>/include/<pkg>.h
```

Windows 下动态库会同时输出导入库（C/C++ 链接需要）：

- MSVC：`.dll` + `.dll.lib`
- GNU/LLVM：`.dll` + `.dll.a`

注意：部分目标（例如 `*-unknown-linux-musl`）可能不支持 `cdylib`。如果你选择了动态库但实际未生成，构建工具会自动回退输出 `staticlib` 并给出提示。

## 交叉编译与 all 构建

- `menu -> all（全部）` 会按内置 target 列表逐个构建
- 若某个 target 缺少 `rust-std`，构建工具会自动执行 `rustup target add <target>`
- Apple 相关 target（macOS/iOS）通常需要 Apple SDK，在非 macOS 主机上会自动跳过
- Windows `*-pc-windows-msvc` 在启用 zigbuild 时可能不兼容：菜单会提示你“保持 MSVC 并关闭 zigbuild”或“切换到 zigbuild 支持的 target”

## 开发与质量检查

```bash
cargo test
cargo clippy --workspace -- -D warnings
```

## 开源协作

- 贡献指南：[CONTRIBUTING.md](CONTRIBUTING.md)
- 行为准则：[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- 安全政策：[SECURITY.md](SECURITY.md)

## 许可证

本项目采用双许可证：Apache-2.0 OR MIT。

- [LICENSE-APACHE](LICENSE-APACHE)
- [LICENSE-MIT](LICENSE-MIT)
