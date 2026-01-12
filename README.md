# ForgeFFI

[English](README.en.md)

ForgeFFI 是一套用 Rust 实现的通用工具库，并提供面向 C/其他语言的 FFI 交付物：

- 动态库 / 静态库
- 自动生成的 C 头文件
- 统一的 `dist/` 产物目录
- 交互式/非交互式构建入口（xtask）

如果你希望把 Rust 的性能与安全带到 C/C++/C#/Java 等语言项目里，同时又不想手工维护繁琐的跨平台构建脚本，这个仓库就是为此准备的。

## 为什么值得关注

- 一条命令生成可分发产物：`.dll/.so/.dylib`、`.a/.lib`、`.h`
- 跨平台更省心：可选 Zig 辅助交叉编译，尽量减少系统工具链差异
- 工程化输出：产物落在稳定的 `dist/<target>/<profile>/...` 结构中，便于 CI/CD 与发布
- 面向开源与协作：双许可证、贡献指南、行为准则、安全政策已就位

## 快速上手

### 1) 交互式构建（推荐）

```bash
cargo xtask menu
```

菜单可选择：profile、模式（模块/聚合、Rust/FFI）、产物类型（动态/静态）、目标平台（包含 `all（全部）`）、是否使用 zigbuild、是否生成头文件。

### 2) 非交互式构建

模块 FFI（以 `net` 动态库为例）：

```bash
cargo xtask build --mode module-ffi --modules net --profile release --artifact cdylib --headers=true
```

聚合 FFI（以 `full` 为例）：

```bash
cargo xtask build --mode aggregate-ffi --features full --profile release --artifact cdylib --headers=true
```

仅下载并打印 Zig 路径（会缓存复用）：

```bash
cargo xtask zig --version 0.12.0
```

## 产物说明（dist）

FFI 构建会把产物复制到 `dist/`：

```
dist/<target>/<debug|release>/<pkg>/<cdylib|staticlib>/...
dist/<target>/<debug|release>/<pkg>/include/<pkg>.h
```

Windows 下动态库会同时输出导入库（C/C++ 链接时需要）：

- MSVC：`.dll` + `.dll.lib`
- GNU/LLVM：`.dll` + `.dll.a`

注意：部分目标（例如 `*-unknown-linux-musl`）可能不支持 `cdylib`。如果你选择了动态库但实际未生成，构建工具会自动回退输出 `staticlib` 并给出提示。

## C 语言调用示例

构建 `tool-net-ffi`（Windows MSVC，release）：

```bash
cargo xtask build --mode module-ffi --modules net --profile release --target x86_64-pc-windows-msvc --zigbuild=false --headers=true
```

然后在 C 项目中：

- include：`dist/x86_64-pc-windows-msvc/release/tool-net-ffi/include/tool-net-ffi.h`
- link：`dist/x86_64-pc-windows-msvc/release/tool-net-ffi/cdylib/tool_net_ffi.dll.lib`
- runtime：`tool_net_ffi.dll` 放到可加载路径

## 交叉编译与 all 构建

- `menu -> all（全部）` 会按内置 target 列表逐个构建。
- 若某个 target 缺少 `rust-std`，构建工具会自动执行 `rustup target add <target>`。
- Apple 相关 target（macOS/iOS）通常需要 Apple SDK，在非 macOS 主机上会自动跳过。
- Windows 的 `*-pc-windows-msvc` 在启用 zigbuild 时可能不兼容：菜单会显式提示你“保持 MSVC 并关闭 zigbuild”或“切换到 zigbuild 支持的 target”。

## 工程结构（面向贡献者）

| crate | 说明 |
| --- | --- |
| `toolbase` | 公共基础能力（其他 crate 复用） |
| `tool-net` / `tool-fs` / `tool-sys` | 各模块 Rust 实现 |
| `tool-net-ffi` / `tool-fs-ffi` / `tool-sys-ffi` | 各模块 FFI 产物（`cdylib`/`staticlib`） |
| `tool` | Rust 聚合 crate（通过 features 组合模块） |
| `tool-ffi` | FFI 聚合 crate（通过 features 组合模块） |
| `xtask` | 构建前端（交互菜单/批量构建/自动下载 Zig/生成头文件） |

## 开发与质量检查

```bash
cargo test
cargo clippy --workspace -- -D warnings
```

## 贡献与安全

- 贡献指南：[CONTRIBUTING.md](CONTRIBUTING.md)
- 行为准则：[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- 安全政策：[SECURITY.md](SECURITY.md)

## 许可证

本项目采用双许可证：Apache-2.0 OR MIT。

- [LICENSE-APACHE](LICENSE-APACHE)
- [LICENSE-MIT](LICENSE-MIT)
