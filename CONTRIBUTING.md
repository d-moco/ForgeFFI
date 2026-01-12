# 贡献指南

欢迎贡献代码、文档与建议。

## 提交前检查

请确保本地通过：

```bash
cargo test
cargo clippy --workspace -- -D warnings
```

## 变更范围

- 新增/修改 FFI API：请同步更新头文件生成相关内容，并保持 ABI 兼容性
- 影响构建流程：请同时验证 `cargo xtask menu` 与 `cargo xtask build` 的非交互路径

## Issue / PR 建议

- 描述清楚目标平台、Rust 版本、是否使用 zigbuild
- 尽量提供可复现的最小步骤与日志

